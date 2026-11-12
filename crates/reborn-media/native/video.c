/* SPDX-License-Identifier: MIT
 *
 * Video for Reborn: a second, picture-only reader of a file whose sound the audio
 * membrane (media.c) plays. FFmpeg demuxes and decodes (H.264 on the Y2's hardware
 * decoder when it takes the stream, else in software); this file turns each picture
 * into a small RGBA image for the 480x360 screen and collects subtitle text.
 *
 * A hardware picture arrives in the decoder's block layout (libavcodec/y2vdec_h264.c:
 * luma in 16x32 blocks, CbCr in 16x16 blocks, format AV_PIX_FMT_DRM_PRIME); software
 * pictures are planar 4:2:0. Both are averaged down in one pass; anything else goes
 * through libswscale.
 */
#define _POSIX_C_SOURCE 200809L
#define _FILE_OFFSET_BITS 64
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/imgutils.h>
#include <libavutil/pixdesc.h>
#include <libswscale/swscale.h>
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define RB_CUES 4096
#define RB_CUE_TEXT 384

typedef struct {
  int64_t start_ms, end_ms;
  char text[RB_CUE_TEXT];
} RbCue;

typedef struct {
  uint32_t width, height;         /* the picture as stored */
  uint32_t out_width, out_height; /* the RGBA image rb_video_convert writes */
  uint32_t hardware;              /* 1: the hardware decoder took the stream */
  uint32_t fps_milli;
  uint32_t subtitles;             /* 1: the file (or a file beside it) has subtitle text */
  uint32_t reserved;
  uint64_t duration_ms;
  char codec[32];
} RbVideoInfo;

typedef struct RbVideo {
  AVFormatContext *f, *sub_file;
  AVCodecContext *c, *sub;
  AVPacket *p;
  AVFrame *frame;
  struct SwsContext *sws;
  int stream, sub_stream, hardware, draining, held;
  int out_w, out_h, step;
  int64_t start_ms;
  uint32_t *sum_y, *sum_u, *sum_v; /* one output row of sums */
  uint16_t *col_y, *col_c;         /* source column -> output column */
  uint32_t *per_y, *per_c;         /* 65536 / samples summed into each output column of a row */
  uint8_t *chroma;                 /* averaged CbCr at half the output size */
  RbCue *cues;
  int cue_count, cue_next;
} RbVideo;

/* The picture codecs Reborn plays (y2_production_defconfig); a cover's are not among them. */
int rb_video_codec(enum AVCodecID id) {
  return id == AV_CODEC_ID_H264 || id == AV_CODEC_ID_MPEG4 || id == AV_CODEC_ID_H263 ||
         id == AV_CODEC_ID_MSMPEG4V3 || id == AV_CODEC_ID_VP8 || id == AV_CODEC_ID_VP9;
}

static enum AVPixelFormat pick_format(AVCodecContext *c, const enum AVPixelFormat *formats) {
  RbVideo *v = c->opaque;
  const enum AVPixelFormat *f;
  if (v->hardware)
    for (f = formats; *f != AV_PIX_FMT_NONE; f++)
      if (*f == AV_PIX_FMT_DRM_PRIME) return *f;
  for (f = formats; *f != AV_PIX_FMT_NONE; f++)
    if (!(av_pix_fmt_desc_get(*f)->flags & AV_PIX_FMT_FLAG_HWACCEL)) return *f;
  return AV_PIX_FMT_NONE;
}

static int text_subtitle(enum AVCodecID id) {
  return id == AV_CODEC_ID_SUBRIP || id == AV_CODEC_ID_ASS || id == AV_CODEC_ID_SSA ||
         id == AV_CODEC_ID_MOV_TEXT || id == AV_CODEC_ID_WEBVTT || id == AV_CODEC_ID_TEXT;
}

/* An ASS event ("ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text") or plain
 * text as lines: override blocks dropped, \N and \n as line breaks, \h as a space. */
static void plain_text(const char *in, int ass, char *out, size_t size) {
  size_t n = 0;
  if (ass)
    for (int commas = 0; *in && commas < 8; in++) commas += *in == ',';
  for (; *in && n + 1 < size; in++) {
    if (*in == '{') {
      const char *close = strchr(in, '}');
      if (close) { in = close; continue; }
    }
    if (*in == '\\' && (in[1] == 'N' || in[1] == 'n')) { out[n++] = '\n'; in++; continue; }
    if (*in == '\\' && in[1] == 'h') { out[n++] = ' '; in++; continue; }
    if (*in == '\r') continue;
    if (*in == '<') { /* <i>, </font …> of SubRip */
      const char *close = strchr(in, '>');
      if (close && close - in < 40) { in = close; continue; }
    }
    out[n++] = *in;
  }
  while (n && (out[n - 1] == '\n' || out[n - 1] == ' ')) n--;
  out[n] = 0;
}

static void add_cue(RbVideo *v, AVCodecContext *decoder, AVPacket *p, AVRational tb) {
  AVSubtitle s;
  int got = 0;
  if (v->cue_count == RB_CUES || avcodec_decode_subtitle2(decoder, &s, &got, p) < 0 || !got) return;
  int64_t at = p->pts == AV_NOPTS_VALUE ? 0 : av_rescale_q(p->pts, tb, (AVRational){1, 1000});
  int64_t length = s.end_display_time > s.start_display_time
      ? (int64_t)s.end_display_time - s.start_display_time
      : av_rescale_q(p->duration, tb, (AVRational){1, 1000});
  RbCue *cue = &v->cues[v->cue_count];
  cue->start_ms = at + s.start_display_time - v->start_ms;
  cue->end_ms = cue->start_ms + (length > 0 ? length : 4000);
  cue->text[0] = 0;
  for (unsigned i = 0; i < s.num_rects; i++) {
    const AVSubtitleRect *r = s.rects[i];
    const char *text = r->type == SUBTITLE_ASS ? r->ass : r->type == SUBTITLE_TEXT ? r->text : NULL;
    size_t used = strlen(cue->text);
    if (!text || used + 2 >= sizeof(cue->text)) continue;
    if (used) cue->text[used++] = '\n';
    plain_text(text, r->type == SUBTITLE_ASS, cue->text + used, sizeof(cue->text) - used);
  }
  avsubtitle_free(&s);
  if (cue->text[0]) v->cue_count++;
}

/* Read the subtitle file `name` (SubRip, ASS, WebVTT) whole, for a film without a text track of
 * its own. Returns the number of lines it gave. */
int rb_video_subtitles(RbVideo *v, const char *name) {
  AVFormatContext *f = NULL;
  AVCodecContext *c = NULL;
  int before = v->cue_count;
  if (v->sub_stream >= 0 || avformat_open_input(&f, name, NULL, NULL) < 0) return 0;
  if (avformat_find_stream_info(f, NULL) >= 0 && f->nb_streams &&
      text_subtitle(f->streams[0]->codecpar->codec_id)) {
    const AVCodec *codec = avcodec_find_decoder(f->streams[0]->codecpar->codec_id);
    c = codec ? avcodec_alloc_context3(codec) : NULL;
    if (c && avcodec_parameters_to_context(c, f->streams[0]->codecpar) >= 0 &&
        avcodec_open2(c, codec, NULL) >= 0) {
      int64_t offset = v->start_ms;
      v->start_ms = 0; /* a file's own times count from the start of the film */
      while (av_read_frame(f, v->p) >= 0) {
        add_cue(v, c, v->p, f->streams[0]->time_base);
        av_packet_unref(v->p);
      }
      v->start_ms = offset;
    }
  }
  avcodec_free_context(&c);
  avformat_close_input(&f);
  return v->cue_count - before;
}

/* A subtitle file beside the video ("film.srt", "film.ass", "film.vtt" for "film.mkv"). */
static void load_subtitle_file(RbVideo *v, const char *path) {
  static const char *const endings[] = {".srt", ".ass", ".ssa", ".vtt"};
  const char *dot = strrchr(path, '.'), *slash = strrchr(path, '/');
  size_t stem = dot && (!slash || dot > slash) ? (size_t)(dot - path) : strlen(path);
  char name[4096];
  if (stem + 8 > sizeof(name)) return;
  for (unsigned i = 0; i < sizeof(endings) / sizeof(*endings) && !v->cue_count; i++) {
    memcpy(name, path, stem);
    strcpy(name + stem, endings[i]);
    rb_video_subtitles(v, name);
  }
}

static void free_scaler(RbVideo *v) {
  free(v->sum_y); free(v->sum_u); free(v->sum_v);
  free(v->col_y); free(v->col_c); free(v->chroma);
  free(v->per_y); free(v->per_c);
  v->sum_y = v->sum_u = v->sum_v = v->per_y = v->per_c = NULL;
  v->col_y = v->col_c = NULL;
  v->chroma = NULL;
}

void rb_video_close(RbVideo *v) {
  if (!v) return;
  free_scaler(v);
  sws_freeContext(v->sws);
  av_frame_free(&v->frame);
  av_packet_free(&v->p);
  avcodec_free_context(&v->sub);
  avcodec_free_context(&v->c);
  avformat_close_input(&v->f);
  free(v->cues);
  free(v);
}

/* The output size: the picture fitted into max_w x max_h in its display shape, never enlarged
 * (the renderer scales a small picture up), both sides even. */
static void output_size(RbVideo *v, int max_w, int max_h) {
  AVRational sar = av_guess_sample_aspect_ratio(v->f, v->f->streams[v->stream], NULL);
  int w = v->c->width, h = v->c->height;
  double shown_w = sar.num > 0 && sar.den > 0 ? (double)w * sar.num / sar.den : w;
  double scale = (double)max_w / shown_w;
  if ((double)max_h / h < scale) scale = (double)max_h / h;
  if (scale > 1 && shown_w <= w) scale = 1;
  v->out_w = ((int)(shown_w * scale + 0.5) + 1) & ~1;
  v->out_h = ((int)(h * scale + 0.5) + 1) & ~1;
  if (v->out_w < 2) v->out_w = 2;
  if (v->out_h < 2) v->out_h = 2;
  if (v->out_w > max_w) v->out_w = max_w & ~1;
  if (v->out_h > max_h) v->out_h = max_h & ~1;
  /* More source than screen: the average needs about four samples for three screen pixels,
   * not every one (1280 wide for 480: every second sample and line; 1920: every third). */
  v->step = w * 3 / (v->out_w * 4);
  if (h * 3 / (v->out_h * 4) < v->step) v->step = h * 3 / (v->out_h * 4);
  if (v->step < 1) v->step = 1;
}

/* 65536 over the number of source columns (every `step`-th) that land in each output column:
 * the average is then a multiplication per pixel, not a division (this CPU divides in software). */
static void column_shares(const uint16_t *col, int w, int step, uint32_t *per, int out) {
  memset(per, 0, (size_t)out * sizeof(*per));
  for (int x = 0; x < w; x += step) per[col[x]]++;
  for (int x = 0; x < out; x++) per[x] = 65536 / (per[x] ? per[x] : 1);
}

static int make_scaler(RbVideo *v) {
  int w = v->c->width, cw = (w + 1) / 2, ow = v->out_w, ocw = ow / 2;
  free_scaler(v);
  v->per_y = calloc((size_t)ow, sizeof(*v->per_y));
  v->per_c = calloc((size_t)ocw, sizeof(*v->per_c));
  v->sum_y = calloc((size_t)ow, sizeof(*v->sum_y));
  v->sum_u = calloc((size_t)ocw, sizeof(*v->sum_u));
  v->sum_v = calloc((size_t)ocw, sizeof(*v->sum_v));
  v->col_y = malloc((size_t)w * sizeof(*v->col_y));
  v->col_c = malloc((size_t)cw * sizeof(*v->col_c));
  v->chroma = malloc((size_t)ocw * (size_t)(v->out_h / 2) * 2);
  if (!v->sum_y || !v->sum_u || !v->sum_v || !v->col_y || !v->col_c || !v->chroma ||
      !v->per_y || !v->per_c)
    return AVERROR(ENOMEM);
  for (int x = 0; x < w; x++) v->col_y[x] = (uint16_t)((int64_t)x * ow / w);
  for (int x = 0; x < cw; x++) v->col_c[x] = (uint16_t)((int64_t)x * ocw / cw);
  column_shares(v->col_y, w, v->step, v->per_y, ow);
  column_shares(v->col_c, cw, v->step, v->per_c, ocw);
  return 0;
}

int rb_video_open(const char *path, int max_w, int max_h, int hardware, RbVideo **out,
                  RbVideoInfo *info) {
  RbVideo *v = calloc(1, sizeof(*v));
  const AVCodec *codec = NULL;
  int ret;
  *out = NULL;
  if (!v) return AVERROR(ENOMEM);
  v->hardware = hardware;
  v->sub_stream = -1;
  v->p = av_packet_alloc();
  v->frame = av_frame_alloc();
  v->cues = calloc(RB_CUES, sizeof(*v->cues));
  if (!v->p || !v->frame || !v->cues) { ret = AVERROR(ENOMEM); goto fail; }
  if ((ret = avformat_open_input(&v->f, path, NULL, NULL)) < 0) goto fail;
  if ((ret = avformat_find_stream_info(v->f, NULL)) < 0) goto fail;
  ret = av_find_best_stream(v->f, AVMEDIA_TYPE_VIDEO, -1, -1, &codec, 0);
  if (ret < 0 || (v->f->streams[ret]->disposition & AV_DISPOSITION_ATTACHED_PIC)) {
    ret = AVERROR_STREAM_NOT_FOUND;
    goto fail;
  }
  v->stream = ret;
  v->start_ms = v->f->start_time == AV_NOPTS_VALUE ? 0 : v->f->start_time / 1000;
  AVStream *st = v->f->streams[v->stream];
  if (!(v->c = avcodec_alloc_context3(codec))) { ret = AVERROR(ENOMEM); goto fail; }
  if ((ret = avcodec_parameters_to_context(v->c, st->codecpar)) < 0) goto fail;
  v->c->opaque = v;
  v->c->get_format = pick_format;
  v->c->pkt_timebase = st->time_base;
  /* Software decoding spreads over the four cores; the hardware path decodes in the calling thread. */
  v->c->thread_count = 4;
  if ((ret = avcodec_open2(v->c, codec, NULL)) < 0) goto fail;
  if (v->c->width < 16 || v->c->height < 16 || v->c->width > 4096 || v->c->height > 4096) {
    ret = AVERROR_PATCHWELCOME;
    goto fail;
  }
  for (unsigned i = 0; i < v->f->nb_streams; i++) {
    AVStream *s = v->f->streams[i];
    if ((int)i == v->stream) continue;
    if (v->sub_stream < 0 && s->codecpar->codec_type == AVMEDIA_TYPE_SUBTITLE &&
        text_subtitle(s->codecpar->codec_id)) {
      const AVCodec *sc = avcodec_find_decoder(s->codecpar->codec_id);
      v->sub = sc ? avcodec_alloc_context3(sc) : NULL;
      if (v->sub && avcodec_parameters_to_context(v->sub, s->codecpar) >= 0 &&
          avcodec_open2(v->sub, sc, NULL) >= 0) {
        v->sub_stream = (int)i;
        continue;
      }
      avcodec_free_context(&v->sub);
    }
    s->discard = AVDISCARD_ALL;
  }
  if (v->sub_stream < 0) load_subtitle_file(v, path);
  output_size(v, max_w, max_h);
  if ((ret = make_scaler(v)) < 0) goto fail;
  memset(info, 0, sizeof(*info));
  info->width = (uint32_t)v->c->width;
  info->height = (uint32_t)v->c->height;
  info->out_width = (uint32_t)v->out_w;
  info->out_height = (uint32_t)v->out_h;
  info->subtitles = v->sub_stream >= 0 || v->cue_count > 0;
  AVRational rate = av_guess_frame_rate(v->f, st, NULL);
  info->fps_milli = rate.num > 0 && rate.den > 0 ? (uint32_t)(1000LL * rate.num / rate.den) : 0;
  info->duration_ms = v->f->duration > 0 ? (uint64_t)(v->f->duration / 1000) : 0;
  snprintf(info->codec, sizeof(info->codec), "%s", avcodec_get_name(v->c->codec_id));
  *out = v;
  return 0;
fail:
  rb_video_close(v);
  return ret;
}

/* Decode the next picture and keep it for rb_video_convert. 1: a picture, `pts_ms` its time from
 * the start of the file; 0: the end; negative: an FFmpeg error. */
int rb_video_next(RbVideo *v, int64_t *pts_ms, uint32_t *hardware) {
  v->held = 0;
  for (;;) {
    int ret = avcodec_receive_frame(v->c, v->frame);
    if (ret >= 0) {
      int64_t ts = v->frame->best_effort_timestamp;
      *pts_ms = ts == AV_NOPTS_VALUE ? -1
          : av_rescale_q(ts, v->f->streams[v->stream]->time_base, (AVRational){1, 1000}) - v->start_ms;
      *hardware = v->frame->format == AV_PIX_FMT_DRM_PRIME;
      v->held = 1;
      return 1;
    }
    if (ret == AVERROR_EOF) return 0;
    if (ret != AVERROR(EAGAIN)) return ret;
    if (v->draining) return 0;
    ret = av_read_frame(v->f, v->p);
    if (ret < 0) {
      v->draining = 1;
      avcodec_send_packet(v->c, NULL);
      continue;
    }
    if (v->p->stream_index == v->stream) {
      ret = avcodec_send_packet(v->c, v->p);
      av_packet_unref(v->p);
      /* A damaged packet costs its picture, not the film. */
      if (ret < 0 && ret != AVERROR(EAGAIN) && ret != AVERROR_INVALIDDATA) return ret;
    } else {
      if (v->p->stream_index == v->sub_stream)
        add_cue(v, v->sub, v->p, v->f->streams[v->sub_stream]->time_base);
      av_packet_unref(v->p);
    }
  }
}

/* Skip pictures nothing refers to while the reader catches up (1), or decode everything (0). */
void rb_video_hurry(RbVideo *v, int on) {
  v->c->skip_frame = on ? AVDISCARD_NONREF : AVDISCARD_DEFAULT;
}

int rb_video_seek(RbVideo *v, int64_t ms) {
  int64_t ts = av_rescale_q(ms + v->start_ms, (AVRational){1, 1000}, v->f->streams[v->stream]->time_base);
  int ret = av_seek_frame(v->f, v->stream, ts, AVSEEK_FLAG_BACKWARD);
  if (ret < 0) return ret;
  avcodec_flush_buffers(v->c);
  v->draining = 0;
  v->held = 0;
  /* Cues of an embedded track are read again from here; a subtitle file's stay. */
  if (v->sub_stream >= 0) {
    avcodec_flush_buffers(v->sub);
    v->cue_count = 0;
  }
  return 0;
}

/* The subtitle text shown at `ms`, "" for none. Returns the text's length. */
int rb_video_subtitle(RbVideo *v, int64_t ms, char *out, int size) {
  if (size < 1) return 0;
  out[0] = 0;
  for (int i = v->cue_count - 1; i >= 0; i--)
    if (v->cues[i].start_ms <= ms && ms < v->cues[i].end_ms) {
      snprintf(out, (size_t)size, "%s", v->cues[i].text);
      break;
    }
  return (int)strlen(out);
}

/* --- pictures to RGBA --- */

typedef struct {
  const AVFrame *f;
  int blocks; /* hardware layout: blocks across; 0 for planes */
} Source;

/* 16 luma samples of row `y` starting at column 16 * `chunk`. */
static inline const uint8_t *luma(const Source *s, int y, int chunk) {
  if (s->blocks)
    return s->f->data[0] + ((size_t)(y / 32 * s->blocks + chunk) * 32 + (size_t)(y % 32)) * 16;
  return s->f->data[0] + (size_t)y * (size_t)s->f->linesize[0] + (size_t)chunk * 16;
}

static void sum_luma_row(const RbVideo *v, const Source *s, int y, int w) {
  int step = v->step;
  for (int chunk = 0; chunk * 16 < w; chunk++) {
    const uint8_t *p = luma(s, y, chunk);
    const uint16_t *col = v->col_y + chunk * 16;
    int n = w - chunk * 16 < 16 ? w - chunk * 16 : 16;
    /* Every `step`-th column of the picture, whichever chunk it falls in. */
    for (int x = (step - chunk * 16 % step) % step; x < n; x += step) v->sum_y[col[x]] += p[x];
  }
}

static void sum_chroma_row(const RbVideo *v, const Source *s, int y, int cw) {
  int step = v->step;
  if (s->blocks) {
    for (int chunk = 0; chunk * 8 < cw; chunk++) {
      const uint8_t *p = s->f->data[1] + ((size_t)(y / 16 * s->blocks + chunk) * 16 + (size_t)(y % 16)) * 16;
      const uint16_t *col = v->col_c + chunk * 8;
      int n = cw - chunk * 8 < 8 ? cw - chunk * 8 : 8;
      for (int x = (step - chunk * 8 % step) % step; x < n; x += step) {
        v->sum_u[col[x]] += p[2 * x];
        v->sum_v[col[x]] += p[2 * x + 1];
      }
    }
    return;
  }
  const uint8_t *u = s->f->data[1] + (size_t)y * (size_t)s->f->linesize[1];
  const uint8_t *w = s->f->data[2] + (size_t)y * (size_t)s->f->linesize[2];
  for (int x = 0; x < cw; x += step) {
    v->sum_u[v->col_c[x]] += u[x];
    v->sum_v[v->col_c[x]] += w[x];
  }
}

/* The average of a column's sum over `rows` lines: sum / (columns * rows) by two multiplications. */
static inline uint32_t average(uint32_t sum, uint32_t per_columns, uint32_t per_rows) {
  return (((sum * per_columns + 32768) >> 16) * per_rows + 32768) >> 16;
}

static inline uint8_t clip(int value) { return value < 0 ? 0 : value > 255 ? 255 : (uint8_t)value; }

static int convert_420(RbVideo *v, const AVFrame *f, uint8_t *rgba) {
  Source s = {f, f->format == AV_PIX_FMT_DRM_PRIME ? f->linesize[0] / 16 : 0};
  int w = v->c->width, h = v->c->height, cw = (w + 1) / 2, ch = (h + 1) / 2;
  int ow = v->out_w, oh = v->out_h, ocw = ow / 2, och = oh / 2, step = v->step;
  if (f->width < w || f->height < h) return AVERROR(EINVAL);
  /* Chroma first: averaged to half the output size. */
  for (int oy = 0; oy < och; oy++) {
    int y0 = (int)((int64_t)oy * ch / och), y1 = (int)((int64_t)(oy + 1) * ch / och), rows = 0;
    memset(v->sum_u, 0, (size_t)ocw * sizeof(*v->sum_u));
    memset(v->sum_v, 0, (size_t)ocw * sizeof(*v->sum_v));
    if (y1 <= y0) y1 = y0 + 1;
    for (int y = y0; y < y1; y += step, rows++) sum_chroma_row(v, &s, y, cw);
    uint8_t *line = v->chroma + (size_t)oy * (size_t)ocw * 2;
    uint32_t per_rows = 65536 / (uint32_t)rows;
    for (int x = 0; x < ocw; x++) {
      line[2 * x] = clip((int)average(v->sum_u[x], v->per_c[x], per_rows));
      line[2 * x + 1] = clip((int)average(v->sum_v[x], v->per_c[x], per_rows));
    }
  }
  /* BT.709 for high definition, BT.601 below, unless the stream says; studio range unless full. */
  int hd = f->colorspace == AVCOL_SPC_BT709 || (f->colorspace == AVCOL_SPC_UNSPECIFIED && h > 576);
  int full = f->color_range == AVCOL_RANGE_JPEG || f->format == AV_PIX_FMT_YUVJ420P;
  int ky = full ? 65536 : 76309, y_off = full ? 0 : 16;
  int rv = hd ? (full ? 103206 : 117489) : (full ? 91881 : 104597);
  int gu = hd ? (full ? 12276 : 13975) : (full ? 22554 : 25675);
  int gv = hd ? (full ? 30679 : 34925) : (full ? 46802 : 53279);
  int bu = hd ? (full ? 121609 : 138438) : (full ? 116130 : 132201);
  for (int oy = 0; oy < oh; oy++) {
    int y0 = (int)((int64_t)oy * h / oh), y1 = (int)((int64_t)(oy + 1) * h / oh), rows = 0;
    memset(v->sum_y, 0, (size_t)ow * sizeof(*v->sum_y));
    if (y1 <= y0) y1 = y0 + 1;
    for (int y = y0; y < y1; y += step, rows++) sum_luma_row(v, &s, y, w);
    const uint8_t *chroma = v->chroma + (size_t)(oy / 2 < och ? oy / 2 : och - 1) * (size_t)ocw * 2;
    uint8_t *out = rgba + (size_t)oy * (size_t)ow * 4;
    uint32_t per_rows = 65536 / (uint32_t)rows;
    for (int x = 0; x < ow; x++) {
      int cx = x / 2 < ocw ? x / 2 : ocw - 1;
      int luma_value = ((int)average(v->sum_y[x], v->per_y[x], per_rows) - y_off) * ky;
      int u = chroma[2 * cx] - 128, w2 = chroma[2 * cx + 1] - 128;
      out[4 * x] = clip((luma_value + rv * w2 + 32768) >> 16);
      out[4 * x + 1] = clip((luma_value - gu * u - gv * w2 + 32768) >> 16);
      out[4 * x + 2] = clip((luma_value + bu * u + 32768) >> 16);
      out[4 * x + 3] = 255;
    }
  }
  return 0;
}

/* Write the held picture as out_width x out_height RGBA. */
int rb_video_convert(RbVideo *v, uint8_t *rgba) {
  const AVFrame *f = v->frame;
  if (!v->held) return AVERROR(EINVAL);
  if (f->format == AV_PIX_FMT_DRM_PRIME || f->format == AV_PIX_FMT_YUV420P ||
      f->format == AV_PIX_FMT_YUVJ420P)
    return convert_420(v, f, rgba);
  v->sws = sws_getCachedContext(v->sws, f->width, f->height, f->format, v->out_w, v->out_h,
                                AV_PIX_FMT_RGBA, SWS_BILINEAR, NULL, NULL, NULL);
  if (!v->sws) return AVERROR(ENOSYS);
  uint8_t *planes[4] = {rgba, NULL, NULL, NULL};
  int strides[4] = {v->out_w * 4, 0, 0, 0};
  sws_scale(v->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, planes, strides);
  return 0;
}

/* A picture of the film for lists and covers: `side` x `side` RGBA, the picture centred on black,
 * taken a tenth into the film (past logos and black leaders). Software decoding only. */
int rb_video_thumbnail(const char *path, uint8_t *rgba, int side) {
  RbVideo *v = NULL;
  RbVideoInfo info;
  int64_t pts;
  uint32_t hardware;
  int ret = rb_video_open(path, side, side, 0, &v, &info);
  if (ret < 0) return ret;
  if (info.duration_ms > 20000) rb_video_seek(v, (int64_t)(info.duration_ms / 10));
  ret = rb_video_next(v, &pts, &hardware);
  if (ret == 0) ret = AVERROR_EOF;
  uint8_t *picture = ret > 0 ? malloc((size_t)v->out_w * (size_t)v->out_h * 4) : NULL;
  if (ret > 0 && !picture) ret = AVERROR(ENOMEM);
  if (ret > 0) ret = rb_video_convert(v, picture);
  if (ret >= 0) {
    int left = (side - v->out_w) / 2, top = (side - v->out_h) / 2;
    for (size_t i = 0; i < (size_t)side * (size_t)side; i++) {
      rgba[4 * i] = rgba[4 * i + 1] = rgba[4 * i + 2] = 0;
      rgba[4 * i + 3] = 255;
    }
    for (int y = 0; y < v->out_h; y++)
      memcpy(rgba + ((size_t)(top + y) * (size_t)side + (size_t)left) * 4,
             picture + (size_t)y * (size_t)v->out_w * 4, (size_t)v->out_w * 4);
    ret = 0;
  }
  free(picture);
  rb_video_close(v);
  return ret;
}

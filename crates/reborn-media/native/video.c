/* SPDX-License-Identifier: MIT
 * Included by media.c: shares descriptor-pinned local I/O, cancellation and limits.
 * One packet, one decoded picture, at most two codec threads; no frame cache here.
 */
typedef struct {
  RbMedia *media;
  struct SwsContext *scale;
  int flushed;
  int64_t origin, next_ms;
} RbVideo;

typedef struct {
  uint64_t duration_ms;
  uint32_t width, height, sar_num, sar_den;
  char codec[64];
} RbVideoInfo;

void rb_video_close(RbVideo *v) {
  if (!v) return;
  sws_freeContext(v->scale);
  rb_media_close(v->media);
  free(v);
}

static int video_stream(RbMedia *m, const AVCodec **codec) {
  for (unsigned i = 0; i < m->f->nb_streams; i++) {
    AVStream *s = m->f->streams[i];
    if (s->codecpar->codec_type == AVMEDIA_TYPE_VIDEO &&
        !(s->disposition & AV_DISPOSITION_ATTACHED_PIC)) {
      *codec = avcodec_find_decoder(s->codecpar->codec_id);
      return *codec ? (int)i : AVERROR_DECODER_NOT_FOUND;
    }
  }
  return AVERROR_STREAM_NOT_FOUND;
}

int rb_video_open(const char *path, RbCancel *cancel, RbVideo **out, RbVideoInfo *info) {
  *out = NULL;
  RbVideo *v = calloc(1, sizeof(*v));
  if (!v) return AVERROR(ENOMEM);
  v->media = calloc(1, sizeof(*v->media));
  int ret = AVERROR(ENOMEM);
  if (!v->media) goto fail;
  RbMedia *m = v->media;
  m->video = 1;
  m->cancel = cancel;
  if ((ret = open_input(m, path, cancel)) < 0) goto fail;
  /* Container headers must supply video geometry. Never let stream probing
   * instantiate an unconstrained decoder before max_pixels is installed. */
  for (unsigned i = 0; i < m->f->nb_streams; i++)
    if (m->f->streams[i]->codecpar->codec_type == AVMEDIA_TYPE_VIDEO)
      m->f->streams[i]->discard = AVDISCARD_ALL;
  if ((ret = avformat_find_stream_info(m->f, NULL)) < 0) goto fail;
  const AVCodec *codec = NULL;
  if ((ret = video_stream(m, &codec)) < 0) goto fail;
  m->stream = ret;
  AVStream *s = m->f->streams[m->stream];
  /* Software decoding on the MT6582: a safety ceiling, not a performance claim.
   * max_pixels also rejects resolution changes in a malicious bitstream. */
  if (s->codecpar->width < 1 || s->codecpar->height < 1 ||
      (int64_t)s->codecpar->width * s->codecpar->height > 720 * 576) {
    ret = AVERROR(EFBIG); goto fail;
  }
  if (av_find_best_stream(m->f, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0) < 0) {
    ret = AVERROR_STREAM_NOT_FOUND; goto fail;
  }
  m->c = avcodec_alloc_context3(codec);
  if (!m->c) { ret = AVERROR(ENOMEM); goto fail; }
  if ((ret = avcodec_parameters_to_context(m->c, s->codecpar)) < 0) goto fail;
  m->c->thread_count = 2;
  m->c->max_pixels = 720 * 576;
  if ((ret = avcodec_open2(m->c, codec, NULL)) < 0) goto fail;
  m->p = av_packet_alloc();
  m->frame = av_frame_alloc();
  if (!m->p || !m->frame) { ret = AVERROR(ENOMEM); goto fail; }
  for (unsigned i = 0; i < m->f->nb_streams; i++)
    m->f->streams[i]->discard = i == (unsigned)m->stream ? AVDISCARD_DEFAULT : AVDISCARD_ALL;
  v->origin = m->f->start_time == AV_NOPTS_VALUE ? 0 : m->f->start_time;
  memset(info, 0, sizeof(*info));
  info->duration_ms = m->f->duration > 0 ? (uint64_t)m->f->duration / 1000 : 0;
  info->width = s->codecpar->width;
  info->height = s->codecpar->height;
  AVRational sar = av_guess_sample_aspect_ratio(m->f, s, NULL);
  info->sar_num = sar.num > 0 ? sar.num : 1;
  info->sar_den = sar.den > 0 ? sar.den : 1;
  snprintf(info->codec, sizeof(info->codec), "%s", codec->name);
  m->io_limited = 0;
  /* Probing deliberately discarded video packets. Rewind the seekable local
   * input before decoding, including short files entirely consumed by analysis. */
  deadline(cancel);
  ret = av_seek_frame(m->f, m->stream,
      av_rescale_q(v->origin, AV_TIME_BASE_Q, s->time_base), AVSEEK_FLAG_BACKWARD);
  if (ret < 0) goto fail;
  *out = v;
  return 0;
fail:
  rb_video_close(v);
  return ret;
}

int rb_video_seek(RbVideo *v, int64_t ms) {
  RbMedia *m = v->media;
  if (ms < 0 || ms > INT64_MAX / 1000 ||
      (v->origin > 0 && ms * 1000 > INT64_MAX - v->origin))
    return AVERROR(EINVAL);
  deadline(m->cancel);
  int64_t target = av_rescale_q(ms * 1000 + v->origin, AV_TIME_BASE_Q,
                               m->f->streams[m->stream]->time_base);
  int ret = av_seek_frame(m->f, m->stream, target, AVSEEK_FLAG_BACKWARD);
  if (ret < 0) return ret;
  avcodec_flush_buffers(m->c);
  av_packet_unref(m->p);
  av_frame_unref(m->frame);
  v->flushed = 0;
  v->next_ms = ms;
  return 0;
}

static int video_receive(RbVideo *v) {
  RbMedia *m = v->media;
  for (;;) {
    if (interrupted(m->cancel)) return AVERROR_EXIT;
    int ret = avcodec_receive_frame(m->c, m->frame);
    if (ret != AVERROR(EAGAIN)) return ret;
    if (v->flushed) return AVERROR_EOF;
    do {
      ret = av_read_frame(m->f, m->p);
      if (ret < 0) {
        if (ret != AVERROR_EOF) return ret;
        v->flushed = 1;
        return avcodec_send_packet(m->c, NULL) < 0 ? AVERROR_INVALIDDATA : video_receive(v);
      }
      if (m->p->stream_index == m->stream) break;
      av_packet_unref(m->p);
    } while (!interrupted(m->cancel));
    ret = avcodec_send_packet(m->c, m->p);
    av_packet_unref(m->p);
    if (ret < 0) return ret;
  }
}

int rb_video_read(RbVideo *v, uint8_t *rgba, int width, int height, int64_t *pts) {
  if (!rgba || width < 1 || height < 1 || width > 480 || height > 360)
    return AVERROR(EINVAL);
  RbMedia *m = v->media;
  deadline(m->cancel);
  int ret = video_receive(v);
  if (ret == AVERROR_EOF) return 0;
  if (ret < 0) return ret;
  AVFrame *f = m->frame;
  if (f->width < 1 || f->height < 1 || (int64_t)f->width * f->height > 720 * 576)
    return AVERROR(EFBIG);
  v->scale = sws_getCachedContext(v->scale, f->width, f->height, f->format,
      width, height, AV_PIX_FMT_RGBA, SWS_FAST_BILINEAR, NULL, NULL, NULL);
  if (!v->scale) return AVERROR(ENOMEM);
  uint8_t *pixels[] = {rgba, NULL, NULL, NULL};
  int strides[] = {width * 4, 0, 0, 0};
  ret = sws_scale(v->scale, (const uint8_t *const *)f->data, f->linesize,
                  0, f->height, pixels, strides);
  if (ret != height) return AVERROR_INVALIDDATA;
  int64_t stamp = f->best_effort_timestamp;
  *pts = stamp == AV_NOPTS_VALUE ? v->next_ms :
      (av_rescale_q(stamp, m->f->streams[m->stream]->time_base, AV_TIME_BASE_Q)
       - v->origin) / 1000;
  AVRational rate = av_guess_frame_rate(m->f, m->f->streams[m->stream], f);
  v->next_ms = *pts + (rate.num > 0 && rate.den > 0 ?
      av_rescale(1000, rate.den, rate.num) : 40);
  av_frame_unref(f);
  return 1;
}

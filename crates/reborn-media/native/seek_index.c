/* SPDX-License-Identifier: MIT
 *
 * Seek points kept across decoders of the same file (Y2Reborn #80).
 *
 * MP3 (FFmpeg's fast seek is off: it lands seconds to minutes off in long
 * VBR files) and raw ADTS AAC have no seek table. FFmpeg finds a position by
 * reading the file from its start, indexing frames on the way. Every resume
 * after standby opens a new decoder, so resuming 5 h into an audiobook read
 * 5 h of file again: seconds on the Y2, longer the further in (M4A and Opus
 * seek by table or bisection and were not affected). A closing decoder
 * leaves a sparse copy of its index here; the next decoder of the same file
 * adds it to its own index before its first seek, and FFmpeg reads only from
 * the nearest point before the target. Memory only: the first seek after
 * Reborn starts still reads from the beginning.
 */
#define _POSIX_C_SOURCE 200809L
#define _FILE_OFFSET_BITS 64
#include "seek_index.h"
#include <pthread.h>
#include <stdlib.h>

/* Files remembered (the least recently used is replaced) and points per
 * file: a 10-hour book keeps a point every ~18 s, so a seek reads a few
 * hundred KB at most. 4 x 2048 points of 16 bytes = 128 KiB. */
#define KEPT_FILES 4
#define KEPT_POINTS 2048
#define KEPT_GAP_MS 5000
/* Shorter files seek quickly by reading, and they must not push a long
 * book out of the four places. */
#define KEPT_MIN_SPAN_MS 120000

typedef struct {
  int64_t pos, timestamp;
} Point;
typedef struct {
  RbFileIdentity id;
  AVRational time_base;
  Point *points;
  int count;
  uint64_t used;
} Kept;

static Kept kept[KEPT_FILES];
static uint64_t kept_clock;
static pthread_mutex_t kept_lock = PTHREAD_MUTEX_INITIALIZER;

void rb_file_identity(RbFileIdentity *id, const struct stat *info) {
  id->dev = info->st_dev;
  id->ino = info->st_ino;
  id->size = info->st_size;
  id->mtime = info->st_mtim;
}
static int same_file(const RbFileIdentity *a, const RbFileIdentity *b) {
  return a->dev == b->dev && a->ino == b->ino && a->size == b->size &&
         a->mtime.tv_sec == b->mtime.tv_sec &&
         a->mtime.tv_nsec == b->mtime.tv_nsec;
}
/* Only demuxers that index while reading need (and fill) these points. */
static AVStream *indexed_stream(AVFormatContext *f, int stream) {
  if (!f || !f->iformat || !(f->iformat->flags & AVFMT_GENERIC_INDEX) ||
      stream < 0 || (unsigned)stream >= f->nb_streams)
    return NULL;
  return f->streams[stream];
}
static Kept *find(const RbFileIdentity *id) {
  for (int i = 0; i < KEPT_FILES; i++)
    if (kept[i].points && same_file(&kept[i].id, id))
      return &kept[i];
  return NULL;
}
static Kept *place(const RbFileIdentity *id) {
  Kept *slot = find(id);
  if (slot)
    return slot;
  slot = &kept[0];
  for (int i = 1; i < KEPT_FILES && slot->points; i++)
    if (!kept[i].points || kept[i].used < slot->used)
      slot = &kept[i];
  return slot;
}

/* The stream's keyframe entries and the points kept before (both sorted by
 * time), merged and thinned to one point per gap. */
typedef struct {
  AVStream *st;
  int index, entries;
  const Point *old;
  int at, count;
} Sources;
static int next_point(Sources *s, Point *out) {
  while (s->index < s->entries) {
    const AVIndexEntry *e = avformat_index_get_entry(s->st, s->index);
    if (e && (e->flags & AVINDEX_KEYFRAME))
      break;
    s->index++;
  }
  const AVIndexEntry *e =
      s->index < s->entries ? avformat_index_get_entry(s->st, s->index) : NULL;
  int old = s->at < s->count;
  if (!e && !old)
    return 0;
  if (e && (!old || e->timestamp <= s->old[s->at].timestamp)) {
    *out = (Point){e->pos, e->timestamp};
    s->index++;
  } else {
    *out = s->old[s->at++];
  }
  return 1;
}
static int merge(Sources s, int64_t gap, Point *out) {
  int count = 0;
  Point p;
  while (count < KEPT_POINTS && next_point(&s, &p))
    if (!count || p.timestamp - out[count - 1].timestamp >= gap)
      out[count++] = p;
  return count;
}
static int span(Sources s, int64_t *first, int64_t *last) {
  Point p;
  int any = 0;
  while (next_point(&s, &p)) {
    if (!any)
      *first = p.timestamp;
    *last = p.timestamp;
    any = 1;
  }
  return any;
}

void rb_seek_points_keep(const RbFileIdentity *id, AVFormatContext *f, int stream) {
  AVStream *st = indexed_stream(f, stream);
  if (!st)
    return;
  pthread_mutex_lock(&kept_lock);
  Kept *slot = place(id);
  int reuse = slot->points && same_file(&slot->id, id) &&
              !av_cmp_q(slot->time_base, st->time_base);
  Sources s = {st, 0, avformat_index_get_entries_count(st),
               reuse ? slot->points : NULL, 0, reuse ? slot->count : 0};
  int64_t first = 0, last = 0;
  AVRational ms = {1, 1000};
  if (span(s, &first, &last) &&
      av_rescale_q(last - first, st->time_base, ms) >= KEPT_MIN_SPAN_MS) {
    int64_t gap = av_rescale_q(KEPT_GAP_MS, ms, st->time_base);
    int64_t even = (last - first) / (KEPT_POINTS - 1) + 1;
    Point *points = malloc(KEPT_POINTS * sizeof(*points));
    if (points) {
      int count = merge(s, gap > even ? gap : even, points);
      free(slot->points);
      *slot = (Kept){*id, st->time_base, points, count, ++kept_clock};
    }
  }
  pthread_mutex_unlock(&kept_lock);
}

void rb_seek_points_restore(const RbFileIdentity *id, AVFormatContext *f, int stream) {
  AVStream *st = indexed_stream(f, stream);
  if (!st)
    return;
  pthread_mutex_lock(&kept_lock);
  Kept *slot = find(id);
  if (slot && !av_cmp_q(slot->time_base, st->time_base)) {
    slot->used = ++kept_clock;
    for (int i = 0; i < slot->count; i++)
      if (slot->points[i].pos >= 0 && slot->points[i].pos < id->size)
        av_add_index_entry(st, slot->points[i].pos, slot->points[i].timestamp,
                           0, 0, AVINDEX_KEYFRAME);
  }
  pthread_mutex_unlock(&kept_lock);
}

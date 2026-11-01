/* SPDX-License-Identifier: MIT
 *
 * Seek points kept across decoders of the same file (seek_index.c).
 */
#ifndef REBORN_SEEK_INDEX_H
#define REBORN_SEEK_INDEX_H
#include <libavformat/avformat.h>
#include <sys/stat.h>
#include <time.h>

/* One file as it is on the card: another file, or this one rewritten, never
 * matches, so kept byte positions are never applied to other contents. */
typedef struct {
  dev_t dev;
  ino_t ino;
  off_t size;
  struct timespec mtime;
} RbFileIdentity;

void rb_file_identity(RbFileIdentity *id, const struct stat *info);
/* A closing decoder leaves the points its index found. */
void rb_seek_points_keep(const RbFileIdentity *id, AVFormatContext *f, int stream);
/* A new decoder of the same file starts its index with them. */
void rb_seek_points_restore(const RbFileIdentity *id, AVFormatContext *f, int stream);
#endif

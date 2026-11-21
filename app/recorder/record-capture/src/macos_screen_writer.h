#ifndef SXC_MACOS_SCREEN_WRITER_H
#define SXC_MACOS_SCREEN_WRITER_H

#include <CoreMedia/CoreMedia.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// The sample belongs to the caller. Append retains it until the ordered writer
// queue has consumed it. All functions except append may wait for file I/O.
void *sxc_screen_writer_create(const char *path, uint32_t width, uint32_t height,
                               uint32_t fps, char *error, size_t error_len);
int sxc_screen_writer_append(void *ctx, CMSampleBufferRef sample,
                             char *error, size_t error_len);
int sxc_screen_writer_finish(void *ctx, int64_t end_pts_ns,
                             char *error, size_t error_len);
int sxc_screen_writer_stats(void *ctx, int64_t *first_pts_ns,
                            int64_t *last_pts_ns, uint64_t *count);
void sxc_screen_writer_destroy(void *ctx);
int sxc_screen_writer_drain_queue(const void *queue, char *error, size_t error_len);
int64_t sxc_screen_writer_clock_now_ns(void);
int64_t sxc_screen_writer_sample_pts_ns(CMSampleBufferRef sample);

#ifdef __cplusplus
}
#endif

#endif

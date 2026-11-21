#ifndef SXC_MACOS_SCREEN_WRITER_FIXTURE_H
#define SXC_MACOS_SCREEN_WRITER_FIXTURE_H

#include <CoreMedia/CoreMedia.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Returns a retained BGRA frame with a visible per-frame colour change.
CMSampleBufferRef sxc_screen_writer_fixture_frame(uint32_t width, uint32_t height,
                                                   int64_t pts_ns, int64_t duration_ns,
                                                   uint32_t frame_number,
                                                   char *error, size_t error_len);
void sxc_screen_writer_fixture_release(CMSampleBufferRef sample);

#ifdef __cplusplus
}
#endif

#endif

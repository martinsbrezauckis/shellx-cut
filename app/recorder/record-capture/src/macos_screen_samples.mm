#import "macos_screen_writer.h"
#import <dispatch/dispatch.h>
#include <cstdio>

static int64_t sample_time_ns(CMTime time) {
    if (!CMTIME_IS_VALID(time) || !CMTIME_IS_NUMERIC(time)) return -1;
    CMTime scaled = CMTimeConvertScale(time, 1000000000, kCMTimeRoundingMethod_RoundHalfAwayFromZero);
    if (!CMTIME_IS_VALID(scaled) || !CMTIME_IS_NUMERIC(scaled) || scaled.value < 0 || scaled.epoch != 0) return -1;
    return scaled.value;
}

extern "C" int64_t sxc_screen_writer_clock_now_ns(void) {
    return sample_time_ns(CMClockGetTime(CMClockGetHostTimeClock()));
}

extern "C" int64_t sxc_screen_writer_sample_pts_ns(CMSampleBufferRef sample) {
    return sample ? sample_time_ns(CMSampleBufferGetPresentationTimeStamp(sample)) : -1;
}

// This fence follows SCK's Stop acknowledgement on the same serial queue that
// delivers screen callbacks. Every queued picture reaches the writer before
// Rust closes admission. A timeout never qualifies a segment.
extern "C" int sxc_screen_writer_drain_queue(const void *queue_ptr, char *error, size_t error_len) {
    if (!queue_ptr) {
        if (error && error_len) std::snprintf(error, error_len, "missing screen callback queue");
        return -1;
    }
    dispatch_queue_t queue = (__bridge dispatch_queue_t)queue_ptr;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    dispatch_async(queue, ^{ dispatch_semaphore_signal(done); });
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 5LL * NSEC_PER_SEC)) != 0) {
        if (error && error_len) std::snprintf(error, error_len, "screen callback queue did not drain within 5 seconds");
        return -1;
    }
    return 0;
}

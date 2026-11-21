#import "macos_screen_writer.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <dispatch/dispatch.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <memory>
#include <mutex>
#include <string>
namespace {
constexpr size_t kMaxQueuedBytes = 256ULL * 1024 * 1024;
constexpr int64_t kNanosecondsPerSecond = 1000000000LL;
constexpr int64_t kFinishTimeoutNanoseconds = 20LL * kNanosecondsPerSecond;

void write_error(char *dst, size_t capacity, const std::string &message) {
    if (dst && capacity) {
        std::snprintf(dst, capacity, "%s", message.c_str());
    }
}

std::string native_error(AVAssetWriter *writer, const char *fallback) {
    NSError *error = writer.error;
    if (error) {
        const char *description = error.localizedDescription.UTF8String;
        if (description && *description) return description;
    }
    return fallback;
}

bool timestamp_ns(CMTime time, int64_t *out) {
    if (!CMTIME_IS_VALID(time) || !CMTIME_IS_NUMERIC(time)) return false;
    CMTime scaled = CMTimeConvertScale(time, static_cast<int32_t>(kNanosecondsPerSecond),
                                      kCMTimeRoundingMethod_RoundHalfAwayFromZero);
    if (!CMTIME_IS_VALID(scaled) || !CMTIME_IS_NUMERIC(scaled) ||
        scaled.value < 0 || scaled.epoch != 0) {
        return false;
    }
    *out = scaled.value;
    return true;
}

CMTime time_from_ns(int64_t ns) {
    return CMTimeMake(ns, static_cast<int32_t>(kNanosecondsPerSecond));
}

struct NativeWriter {
    std::mutex mutex;
    __strong AVAssetWriter *writer = nil;
    __strong AVAssetWriterInput *video = nil;
    __strong dispatch_queue_t queue = nil;
    __strong dispatch_semaphore_t finished_signal = nil;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t fps = 0;
    size_t queued_bytes = 0;
    int64_t first_pts_ns = -1;
    int64_t last_pts_ns = -1;
    int64_t last_queued_pts_ns = -1;
    uint64_t frame_count = 0;
    bool started = false;
    bool finishing = false;
    bool finished = false;
    std::string failure;

    void fail(const std::string &message) {
        std::lock_guard<std::mutex> guard(mutex);
        if (failure.empty()) failure = message;
    }

    void append_on_queue(CMSampleBufferRef sample, size_t bytes, int64_t pts_ns) {
        @autoreleasepool {
            {
                std::lock_guard<std::mutex> guard(mutex);
                if (!failure.empty()) {
                    queued_bytes -= bytes;
                    CFRelease(sample);
                    return;
                }
            }
            if (!started) {
                if (![writer startWriting]) {
                    fail(native_error(writer, "could not start the video writer"));
                } else {
                    [writer startSessionAtSourceTime:time_from_ns(pts_ns)];
                    started = true;
                }
            }
            if (started) {
                const auto deadline = std::chrono::steady_clock::now() +
                                      std::chrono::seconds(20);
                while (!video.readyForMoreMediaData &&
                       writer.status == AVAssetWriterStatusWriting &&
                       std::chrono::steady_clock::now() < deadline) {
                    // This is the writer queue, never the ScreenCaptureKit callback.
                    [NSThread sleepForTimeInterval:0.002];
                }
                if (!video.readyForMoreMediaData) {
                    fail(native_error(writer, "video encoder did not accept a frame in time"));
                } else if (![video appendSampleBuffer:sample]) {
                    fail(native_error(writer, "video encoder rejected a screen frame"));
                } else {
                    std::lock_guard<std::mutex> guard(mutex);
                    if (first_pts_ns < 0) first_pts_ns = pts_ns;
                    last_pts_ns = pts_ns;
                    ++frame_count;
                }
            }
            {
                std::lock_guard<std::mutex> guard(mutex);
                queued_bytes -= bytes;
            }
            CFRelease(sample);
        }
    }

    void finish_on_queue(std::shared_ptr<NativeWriter> self, int64_t end_pts_ns) {
        @autoreleasepool {
            bool has_frames;
            bool failed;
            int64_t last_pts;
            {
                std::lock_guard<std::mutex> guard(mutex);
                has_frames = frame_count > 0;
                failed = !failure.empty();
                last_pts = last_pts_ns;
            }
            if (!has_frames) {
                fail("screen segment contains no picture");
                [writer cancelWriting];
            } else if (failed) {
                [writer cancelWriting];
            } else {
                if (end_pts_ns > last_pts) {
                    [writer endSessionAtSourceTime:time_from_ns(end_pts_ns)];
                }
                [video markAsFinished];
                [writer finishWritingWithCompletionHandler:^{
                    if (writer.status != AVAssetWriterStatusCompleted) {
                        self->fail(native_error(writer, "video writer did not finish the segment"));
                    }
                    {
                        std::lock_guard<std::mutex> guard(self->mutex);
                        self->finished = true;
                    }
                    dispatch_semaphore_signal(self->finished_signal);
                }];
                return;
            }
            {
                std::lock_guard<std::mutex> guard(mutex);
                finished = true;
            }
            dispatch_semaphore_signal(finished_signal);
        }
    }
};

struct WriterHandle {
    std::shared_ptr<NativeWriter> state;
};
}  // namespace

extern "C" void *sxc_screen_writer_create(const char *path, uint32_t width,
                                            uint32_t height, uint32_t fps,
                                            char *error, size_t error_len) {
    @autoreleasepool {
        if (!path || !*path || width < 2 || height < 2 ||
            width > 16384 || height > 16384 || !fps || fps > 240) {
            write_error(error, error_len, "invalid video writer path or dimensions");
            return nullptr;
        }
        NSString *file = [NSString stringWithUTF8String:path];
        if (!file) {
            write_error(error, error_len, "video writer path is not UTF-8");
            return nullptr;
        }
        NSURL *url = [NSURL fileURLWithPath:file];
        NSError *creation_error = nil;
        AVAssetWriter *writer = [[AVAssetWriter alloc] initWithURL:url
                                                         fileType:AVFileTypeMPEG4
                                                            error:&creation_error];
        if (!writer) {
            write_error(error, error_len,
                        creation_error.localizedDescription.UTF8String ?: "could not create video writer");
            return nullptr;
        }
        NSDictionary *compression = @{
            AVVideoAverageBitRateKey: @(std::clamp<uint64_t>(
                static_cast<uint64_t>(width) * height * fps / 8, 4000000, 80000000)),
            AVVideoExpectedSourceFrameRateKey: @(fps),
            AVVideoMaxKeyFrameIntervalKey: @(fps * 2),
            AVVideoAllowFrameReorderingKey: @NO
        };
        NSDictionary *settings = @{
            AVVideoCodecKey: AVVideoCodecTypeH264,
            AVVideoWidthKey: @(width),
            AVVideoHeightKey: @(height),
            AVVideoCompressionPropertiesKey: compression
        };
        AVAssetWriterInput *video = [[AVAssetWriterInput alloc] initWithMediaType:AVMediaTypeVideo
                                                                  outputSettings:settings];
        video.expectsMediaDataInRealTime = YES;
        if (![writer canAddInput:video]) {
            write_error(error, error_len, "video writer rejected H.264 input");
            return nullptr;
        }
        [writer addInput:video];
        writer.movieFragmentInterval = CMTimeMake(1, 1);
        auto state = std::make_shared<NativeWriter>();
        state->writer = writer;
        state->video = video;
        state->width = width;
        state->height = height;
        state->fps = fps;
        state->queue = dispatch_queue_create("cut.screen.writer", DISPATCH_QUEUE_SERIAL);
        state->finished_signal = dispatch_semaphore_create(0);
        return new WriterHandle{std::move(state)};
    }
}

extern "C" int sxc_screen_writer_append(void *ctx, CMSampleBufferRef sample,
                                          char *error, size_t error_len) {
    if (!ctx || !sample) {
        write_error(error, error_len, "missing video writer or screen sample");
        return -1;
    }
    CVImageBufferRef image = CMSampleBufferGetImageBuffer(sample);
    if (!image) {
        write_error(error, error_len, "screen sample has no image buffer");
        return -1;
    }
    auto state = static_cast<WriterHandle *>(ctx)->state;
    if (CVPixelBufferGetWidth(image) != state->width ||
        CVPixelBufferGetHeight(image) != state->height) {
        write_error(error, error_len, "screen sample dimensions changed");
        return -1;
    }
    int64_t pts_ns;
    if (!timestamp_ns(CMSampleBufferGetPresentationTimeStamp(sample), &pts_ns)) {
        write_error(error, error_len, "screen sample has no valid presentation time");
        return -1;
    }
    size_t bytes = CVPixelBufferGetDataSize(image);
    if (!bytes || bytes > kMaxQueuedBytes) {
        write_error(error, error_len, "screen sample exceeds the writer queue limit");
        return -1;
    }
    {
        std::lock_guard<std::mutex> guard(state->mutex);
        if (!state->failure.empty()) {
            write_error(error, error_len, state->failure);
            return -1;
        }
        if (state->finishing) {
            write_error(error, error_len, "screen segment is already finishing");
            return -1;
        }
        if (state->last_queued_pts_ns >= pts_ns) {
            write_error(error, error_len, "screen sample time moved backwards");
            return -1;
        }
        if (bytes > kMaxQueuedBytes - state->queued_bytes) {
            write_error(error, error_len, "screen writer queue is full");
            return -1;
        }
        state->last_queued_pts_ns = pts_ns;
        state->queued_bytes += bytes;
        CFRetain(sample);
        dispatch_async(state->queue, ^{
            state->append_on_queue(sample, bytes, pts_ns);
        });
    }
    return 0;
}

extern "C" int sxc_screen_writer_finish(void *ctx, int64_t end_pts_ns,
                                          char *error, size_t error_len) {
    if (!ctx) {
        write_error(error, error_len, "missing video writer");
        return -1;
    }
    auto state = static_cast<WriterHandle *>(ctx)->state;
    {
        std::lock_guard<std::mutex> guard(state->mutex);
        if (!state->finishing) {
            state->finishing = true;
            dispatch_async(state->queue, ^{
                state->finish_on_queue(state, end_pts_ns);
            });
        }
    }
    int64_t wait = dispatch_semaphore_wait(
        state->finished_signal,
        dispatch_time(DISPATCH_TIME_NOW, kFinishTimeoutNanoseconds));
    if (wait != 0) {
        write_error(error, error_len, "video writer did not finish within 20 seconds");
        return -1;
    }
    std::lock_guard<std::mutex> guard(state->mutex);
    // A retry after an earlier timeout must observe the completed state.
    if (state->finished) dispatch_semaphore_signal(state->finished_signal);
    if (!state->failure.empty()) {
        write_error(error, error_len, state->failure);
        return -1;
    }
    return 0;
}

extern "C" int sxc_screen_writer_stats(void *ctx, int64_t *first_pts_ns,
                                         int64_t *last_pts_ns, uint64_t *count) {
    if (!ctx || !first_pts_ns || !last_pts_ns || !count) return -1;
    auto state = static_cast<WriterHandle *>(ctx)->state;
    std::lock_guard<std::mutex> guard(state->mutex);
    if (!state->finished || !state->failure.empty()) return -1;
    *first_pts_ns = state->first_pts_ns;
    *last_pts_ns = state->last_pts_ns;
    *count = state->frame_count;
    return 0;
}

extern "C" void sxc_screen_writer_destroy(void *ctx) {
    if (!ctx) return;
    auto *handle = static_cast<WriterHandle *>(ctx);
    auto state = handle->state;
    {
        std::lock_guard<std::mutex> guard(state->mutex);
        if (!state->finishing) {
            state->finishing = true;
            // This runs after every accepted sample. A failed or abandoned
            // capture must release the encoder without racing its own queue.
            dispatch_async(state->queue, ^{
                @autoreleasepool {
                    if (state->writer.status == AVAssetWriterStatusWriting) {
                        [state->writer cancelWriting];
                    }
                    {
                        std::lock_guard<std::mutex> done(state->mutex);
                        state->finished = true;
                    }
                    dispatch_semaphore_signal(state->finished_signal);
                }
            });
        }
    }
    delete handle;
}

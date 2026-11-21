#import "macos_screen_writer_fixture.h"

#import <CoreVideo/CoreVideo.h>

#include <cstdio>
#include <cstring>

namespace {
constexpr int32_t kNanosecondsPerSecond = 1000000000;

void error_text(char *dst, size_t capacity, const char *message) {
    if (dst && capacity) std::snprintf(dst, capacity, "%s", message);
}
}

extern "C" CMSampleBufferRef sxc_screen_writer_fixture_frame(
    uint32_t width, uint32_t height, int64_t pts_ns, int64_t duration_ns,
    uint32_t frame_number, char *error, size_t error_len) {
    if (!width || !height || width > 8192 || height > 8192 ||
        pts_ns < 0 || duration_ns <= 0) {
        error_text(error, error_len, "invalid fixture frame dimensions or time");
        return nullptr;
    }
    CVPixelBufferRef pixel = nullptr;
    CVReturn created = CVPixelBufferCreate(
        kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
        nullptr, &pixel);
    if (created != kCVReturnSuccess || !pixel) {
        error_text(error, error_len, "could not allocate fixture frame");
        return nullptr;
    }
    if (CVPixelBufferLockBaseAddress(pixel, 0) != kCVReturnSuccess) {
        CVPixelBufferRelease(pixel);
        error_text(error, error_len, "could not lock fixture frame");
        return nullptr;
    }
    auto *base = static_cast<uint8_t *>(CVPixelBufferGetBaseAddress(pixel));
    size_t row_bytes = CVPixelBufferGetBytesPerRow(pixel);
    // A moving horizontal gradient fills one scanline, then copies it. This
    // keeps the fixture producer faster than 4K60 without changing its sample
    // dimensions, byte ownership, timestamps or per-frame picture identity.
    for (uint32_t x = 0; x < width; ++x) {
        size_t offset = static_cast<size_t>(x) * 4;
        base[offset] = static_cast<uint8_t>(frame_number);
        base[offset + 1] = static_cast<uint8_t>(frame_number >> 8);
        base[offset + 2] = static_cast<uint8_t>((frame_number * 17 + x) & 0xff);
        base[offset + 3] = 255;
    }
    for (uint32_t y = 1; y < height; ++y) {
        std::memcpy(base + static_cast<size_t>(y) * row_bytes, base, static_cast<size_t>(width) * 4);
    }
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    CMVideoFormatDescriptionRef format = nullptr;
    OSStatus described = CMVideoFormatDescriptionCreateForImageBuffer(
        kCFAllocatorDefault, pixel, &format);
    if (described != noErr || !format) {
        CVPixelBufferRelease(pixel);
        error_text(error, error_len, "could not describe fixture frame");
        return nullptr;
    }
    CMSampleTimingInfo timing = {
        CMTimeMake(duration_ns, kNanosecondsPerSecond),
        CMTimeMake(pts_ns, kNanosecondsPerSecond),
        kCMTimeInvalid
    };
    CMSampleBufferRef sample = nullptr;
    OSStatus sampled = CMSampleBufferCreateReadyWithImageBuffer(
        kCFAllocatorDefault, pixel, format, &timing, &sample);
    CFRelease(format);
    CVPixelBufferRelease(pixel);
    if (sampled != noErr || !sample) {
        error_text(error, error_len, "could not create fixture screen sample");
        return nullptr;
    }
    return sample;
}

extern "C" void sxc_screen_writer_fixture_release(CMSampleBufferRef sample) {
    if (sample) CFRelease(sample);
}

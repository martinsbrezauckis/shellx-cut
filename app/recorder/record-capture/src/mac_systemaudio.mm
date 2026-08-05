// mac_systemaudio.mm — DESKTOP/SYSTEM audio capture via the macOS 14.2+ Core Audio
// process-tap API: AudioHardwareCreateProcessTap (a stereo GLOBAL tap) + a private
// aggregate device that contains the tap + an IOProc that drains the tap's audio.
//
// This uses a standard Core Audio process-tap technique. It replaces
// the ScreenCaptureKit `capturesAudio` path, which silently delivers ZERO audio buffers on
// macOS 15+/26 (the audio sample callback is never invoked in the affected configuration).
// The containing app supplies NSAudioCaptureUsageDescription so the first aggregate-device
// start can request system-audio permission. Reference: Apple's "Capturing system audio with
// Core Audio taps" sample.
//
// Exposed to the Rust recorder (macos.rs) via a tiny C ABI:
//   void*  sxc_sysaudio_start(void)                          -> opaque ctx (NULL on failure)
//   int    sxc_sysaudio_stop(ctx, &samples,&n,&channels,&rate) -> 0 ok; mallocs interleaved f32
//   void   sxc_sysaudio_free(samples)
// The Rust side converts the f32 PCM to a 16-bit `system.wav` (the a_system track).

#import <Foundation/Foundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreAudio/AudioHardwareTapping.h> // AudioHardwareCreate/DestroyProcessTap
#import <CoreAudio/CATapDescription.h>
#include <vector>
#include <mutex>
#include <cstring>
#include <cstdlib>

namespace {

struct TapCtx {
    AudioObjectID tapID = kAudioObjectUnknown;
    AudioDeviceID aggID = kAudioObjectUnknown;
    AudioDeviceIOProcID procID = nullptr;
    std::mutex mtx;
    std::vector<float> samples; // interleaved
    uint32_t channels = 2;
    double rate = 48000.0;
};

// IOProc: drains the tap's audio into the accumulator. Runs on a Core Audio thread; the
// mutex is uncontended during capture (Rust only locks at stop) so it never glitches.
OSStatus tap_ioproc(AudioObjectID inDevice, const AudioTimeStamp* inNow,
                    const AudioBufferList* inInputData, const AudioTimeStamp* inInputTime,
                    AudioBufferList* outOutputData, const AudioTimeStamp* inOutputTime,
                    void* inClientData) {
    (void)inDevice; (void)inNow; (void)inInputTime; (void)outOutputData; (void)inOutputTime;
    TapCtx* c = static_cast<TapCtx*>(inClientData);
    if (!c || !inInputData || inInputData->mNumberBuffers == 0) return noErr;
    std::lock_guard<std::mutex> lk(c->mtx);
    const AudioBufferList* bl = inInputData;
    if (bl->mNumberBuffers >= 2 && bl->mBuffers[0].mNumberChannels == 1) {
        // Planar (one mono buffer per channel) — interleave L/R.
        const float* L = static_cast<const float*>(bl->mBuffers[0].mData);
        const float* R = static_cast<const float*>(bl->mBuffers[1].mData);
        if (L && R) {
            size_t frames = bl->mBuffers[0].mDataByteSize / sizeof(float);
            c->channels = 2;
            c->samples.reserve(c->samples.size() + frames * 2);
            for (size_t f = 0; f < frames; f++) { c->samples.push_back(L[f]); c->samples.push_back(R[f]); }
        }
    } else {
        // Single buffer — already interleaved (mNumberChannels wide) or true mono.
        const AudioBuffer* b = &bl->mBuffers[0];
        if (b->mData) {
            if (b->mNumberChannels) c->channels = b->mNumberChannels;
            const float* d = static_cast<const float*>(b->mData);
            size_t n = b->mDataByteSize / sizeof(float);
            c->samples.insert(c->samples.end(), d, d + n);
        }
    }
    return noErr;
}

// UID (CFString, caller releases) of the default system OUTPUT device — the aggregate
// device's main sub-device, so the tap mixes down the audio routed there.
CFStringRef copy_default_output_uid(void) {
    AudioObjectID dev = kAudioObjectUnknown;
    UInt32 sz = sizeof(dev);
    AudioObjectPropertyAddress a = { kAudioHardwarePropertyDefaultSystemOutputDevice,
                                     kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &a, 0, NULL, &sz, &dev) != noErr ||
        dev == kAudioObjectUnknown) return NULL;
    CFStringRef uid = NULL; UInt32 usz = sizeof(uid);
    AudioObjectPropertyAddress u = { kAudioDevicePropertyDeviceUID,
                                     kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
    if (AudioObjectGetPropertyData(dev, &u, 0, NULL, &usz, &uid) != noErr) return NULL;
    return uid;
}

// Apple documents the aggregate tap list as a CFArray of the HAL-reported tap UIDs.
// CATapDescription.UUID identifies the description, but it is not the property contract for
// kAudioAggregateDevicePropertyTapList. Using the actual kAudioTapPropertyUID also makes the
// aggregate start the point where macOS requests system-audio permission.
bool attach_tap_to_aggregate(AudioObjectID tapID, AudioObjectID aggID) {
    CFStringRef tapUID = NULL;
    UInt32 uidSize = sizeof(tapUID);
    AudioObjectPropertyAddress uidAddress = { kAudioTapPropertyUID,
                                               kAudioObjectPropertyScopeGlobal,
                                               kAudioObjectPropertyElementMain };
    OSStatus st = AudioObjectGetPropertyData(tapID, &uidAddress, 0, NULL, &uidSize, &tapUID);
    if (st != noErr || !tapUID) return false;

    const void* values[] = { tapUID };
    CFArrayRef tapList = CFArrayCreate(kCFAllocatorDefault, values, 1, &kCFTypeArrayCallBacks);
    if (!tapList) return false;

    AudioObjectPropertyAddress listAddress = { kAudioAggregateDevicePropertyTapList,
                                                kAudioObjectPropertyScopeGlobal,
                                                kAudioObjectPropertyElementMain };
    UInt32 listSize = sizeof(CFStringRef);
    st = AudioObjectSetPropertyData(aggID, &listAddress, 0, NULL, listSize, &tapList);
    CFRelease(tapList);
    return st == noErr;
}

} // namespace

extern "C" void* sxc_sysaudio_start(void) {
    @autoreleasepool {
        TapCtx* c = new TapCtx();

        // 1) Stereo GLOBAL tap excluding NO processes = capture ALL system audio. Don't mute
        //    the output (the user still hears the audio while it's tapped).
        CATapDescription* desc = [[CATapDescription alloc] initStereoGlobalTapButExcludeProcesses:@[]];
        desc.name = @"ShellX Cut System Audio";
        desc.UUID = [NSUUID UUID];
        desc.muteBehavior = CATapUnmuted;

        AudioObjectID tapID = kAudioObjectUnknown;
        OSStatus st = AudioHardwareCreateProcessTap(desc, &tapID);
        if (st != noErr || tapID == kAudioObjectUnknown) { delete c; return NULL; }
        c->tapID = tapID;

        // 2) Tap stream format (rate/channels) — best-effort; keep the 48 kHz stereo default.
        AudioStreamBasicDescription asbd; memset(&asbd, 0, sizeof(asbd));
        UInt32 fsz = sizeof(asbd);
        AudioObjectPropertyAddress fmt = { kAudioTapPropertyFormat,
                                           kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
        if (AudioObjectGetPropertyData(tapID, &fmt, 0, NULL, &fsz, &asbd) == noErr && asbd.mSampleRate > 0) {
            c->rate = asbd.mSampleRate;
            if (asbd.mChannelsPerFrame > 0) c->channels = asbd.mChannelsPerFrame;
        }

        // 3) Private aggregate device that contains the tap (the only way to read a tap's audio).
        CFStringRef outUID = copy_default_output_uid();
        NSString* aggUID = [[NSUUID UUID] UUIDString];
        // NB: the CoreAudio aggregate/tap dictionary keys are plain C-string #defines
        // (const char*), so they're boxed with @(...) into NSStrings — NOT __bridge'd.
        // outUID is a real CFStringRef, so it IS __bridge'd.
        NSMutableDictionary* d = [@{
            @(kAudioAggregateDeviceNameKey): @"ShellX Cut Aggregate",
            @(kAudioAggregateDeviceUIDKey): aggUID,
            @(kAudioAggregateDeviceIsPrivateKey): @YES,
            @(kAudioAggregateDeviceIsStackedKey): @NO,
            @(kAudioAggregateDeviceTapAutoStartKey): @YES,
        } mutableCopy];
        if (outUID) {
            d[@(kAudioAggregateDeviceMainSubDeviceKey)] = (__bridge NSString*)outUID;
            d[@(kAudioAggregateDeviceSubDeviceListKey)] = @[ @{
                @(kAudioSubDeviceUIDKey): (__bridge NSString*)outUID,
            } ];
        }
        AudioObjectID aggID = kAudioObjectUnknown;
        st = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)d, &aggID);
        if (outUID) CFRelease(outUID);
        if (st != noErr || aggID == kAudioObjectUnknown) {
            AudioHardwareDestroyProcessTap(tapID);
            delete c; return NULL;
        }
        c->aggID = aggID;

        if (!attach_tap_to_aggregate(tapID, aggID)) {
            AudioHardwareDestroyAggregateDevice(aggID);
            AudioHardwareDestroyProcessTap(tapID);
            delete c; return NULL;
        }

        // 4) IOProc + start. The aggregate auto-starts the tap (TapAutoStart) and the IOProc
        //    receives the tap's audio in its input buffer list.
        st = AudioDeviceCreateIOProcID(aggID, tap_ioproc, c, &c->procID);
        if (st != noErr || !c->procID) {
            AudioHardwareDestroyAggregateDevice(aggID);
            AudioHardwareDestroyProcessTap(tapID);
            delete c; return NULL;
        }
        st = AudioDeviceStart(aggID, c->procID);
        if (st != noErr) {
            AudioDeviceDestroyIOProcID(aggID, c->procID);
            AudioHardwareDestroyAggregateDevice(aggID);
            AudioHardwareDestroyProcessTap(tapID);
            delete c; return NULL;
        }
        return c;
    }
}

extern "C" int sxc_sysaudio_stop(void* h, float** out_samples, uint64_t* out_count,
                                 uint32_t* out_channels, double* out_rate) {
    if (out_samples) *out_samples = nullptr;
    if (out_count) *out_count = 0;
    if (out_channels) *out_channels = 0;
    if (out_rate) *out_rate = 0.0;
    if (!h) return -1;
    TapCtx* c = static_cast<TapCtx*>(h);
    if (c->aggID != kAudioObjectUnknown && c->procID) {
        AudioDeviceStop(c->aggID, c->procID);
        AudioDeviceDestroyIOProcID(c->aggID, c->procID);
    }
    if (c->aggID != kAudioObjectUnknown) AudioHardwareDestroyAggregateDevice(c->aggID);
    if (c->tapID != kAudioObjectUnknown) AudioHardwareDestroyProcessTap(c->tapID);

    std::vector<float> samples;
    uint32_t channels = 2;
    double rate = 48000.0;
    {
        std::lock_guard<std::mutex> lk(c->mtx);
        samples.swap(c->samples);
        channels = c->channels ? c->channels : 2;
        rate = c->rate > 0 ? c->rate : 48000.0;
    }
    delete c;

    // A null sample output is the Rust RAII guard's teardown-only path.
    if (!out_samples) return 0;
    if (!out_count || !out_channels || !out_rate) return -2;

    const size_t n = samples.size();
    if (n > SIZE_MAX / sizeof(float)) return -3;
    *out_count = static_cast<uint64_t>(n);
    *out_channels = channels;
    *out_rate = rate;
    if (!n) return 0;

    float* buf = static_cast<float*>(std::malloc(n * sizeof(float)));
    if (!buf) {
        *out_count = 0;
        return -4;
    }
    std::memcpy(buf, samples.data(), n * sizeof(float));
    *out_samples = buf;
    return 0;
}

extern "C" void sxc_sysaudio_free(float* p) { if (p) std::free(p); }

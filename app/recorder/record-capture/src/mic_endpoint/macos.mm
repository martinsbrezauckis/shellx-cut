#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>

#include <stdlib.h>
#include <string.h>

extern "C" char* shellx_cut_mic_endpoints_json(void) {
    @autoreleasepool {
        AudioObjectPropertyAddress listAddress = {
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        };
        UInt32 size = 0;
        if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &listAddress, 0, NULL, &size) != noErr ||
            size == 0 || size % sizeof(AudioDeviceID) != 0) {
            return NULL;
        }
        size_t count = size / sizeof(AudioDeviceID);
        AudioDeviceID* devices = (AudioDeviceID*)calloc(count, sizeof(AudioDeviceID));
        if (!devices) return NULL;
        if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &listAddress, 0, NULL, &size, devices) != noErr) {
            free(devices);
            return NULL;
        }

        NSMutableArray* rows = [NSMutableArray array];
        for (size_t index = 0; index < count; ++index) {
            AudioObjectID device = devices[index];
            AudioObjectPropertyAddress streamAddress = {
                kAudioDevicePropertyStreamConfiguration,
                kAudioDevicePropertyScopeInput,
                kAudioObjectPropertyElementMain,
            };
            UInt32 streamSize = 0;
            if (AudioObjectGetPropertyDataSize(device, &streamAddress, 0, NULL, &streamSize) != noErr ||
                streamSize < sizeof(AudioBufferList)) {
                continue;
            }
            AudioBufferList* streams = (AudioBufferList*)malloc(streamSize);
            if (!streams) continue;
            bool hasInput = false;
            if (AudioObjectGetPropertyData(device, &streamAddress, 0, NULL, &streamSize, streams) == noErr) {
                for (UInt32 buffer = 0; buffer < streams->mNumberBuffers; ++buffer) {
                    if (streams->mBuffers[buffer].mNumberChannels > 0) {
                        hasInput = true;
                        break;
                    }
                }
            }
            free(streams);
            if (!hasInput) continue;

            AudioObjectPropertyAddress uidAddress = {
                kAudioDevicePropertyDeviceUID,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            };
            AudioObjectPropertyAddress nameAddress = {
                kAudioObjectPropertyName,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            };
            CFStringRef uid = NULL;
            CFStringRef name = NULL;
            UInt32 stringSize = sizeof(CFStringRef);
            if (AudioObjectGetPropertyData(device, &uidAddress, 0, NULL, &stringSize, &uid) != noErr || !uid) {
                continue;
            }
            stringSize = sizeof(CFStringRef);
            if (AudioObjectGetPropertyData(device, &nameAddress, 0, NULL, &stringSize, &name) != noErr || !name) {
                CFRelease(uid);
                continue;
            }
            [rows addObject:@{
                @"uid": (__bridge NSString*)uid,
                @"name": (__bridge NSString*)name,
            }];
            CFRelease(uid);
            CFRelease(name);
        }
        free(devices);

        NSError* error = nil;
        NSData* json = [NSJSONSerialization dataWithJSONObject:rows options:0 error:&error];
        if (!json || error) return NULL;
        char* output = (char*)malloc(json.length + 1);
        if (!output) return NULL;
        memcpy(output, json.bytes, json.length);
        output[json.length] = '\0';
        return output;
    }
}

extern "C" void shellx_cut_free_mic_endpoints_json(char* value) {
    free(value);
}

// Microphone permission as AVFoundation reports it, read without a prompt:
// 2 granted, 0 not answered yet, 1 denied, 3 restricted.
extern "C" int32_t shellx_cut_microphone_authorization(void) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]) {
        case AVAuthorizationStatusAuthorized: return 2;
        case AVAuthorizationStatusDenied: return 1;
        case AVAuthorizationStatusRestricted: return 3;
        case AVAuthorizationStatusNotDetermined: return 0;
    }
    return 3;
}

// Show the macOS microphone prompt when the person has not answered yet. The
// answer arrives asynchronously; the caller reads the state again later.
extern "C" void shellx_cut_request_microphone_access(void) {
    if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio] != AVAuthorizationStatusNotDetermined) return;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL) {}];
}

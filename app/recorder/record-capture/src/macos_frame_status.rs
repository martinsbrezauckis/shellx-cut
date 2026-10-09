//! Read the actual SCK status attachment; the locked Swift bridge casts its
//! NSNumber to SCFrameStatus and consequently drops this field.
#[derive(Debug, Clone, Copy)]
enum StatusNumber {
    Missing,
    WrongType,
    Floating,
    ConversionFailed,
    Integer(i64),
}

fn decode_status(value: StatusNumber) -> Result<i32, &'static str> {
    match value {
        StatusNumber::Missing => Err("missing SCK status attachment"),
        StatusNumber::WrongType => Err("SCK status attachment is not CFNumber"),
        StatusNumber::Floating => Err("SCK status attachment is not an integer"),
        StatusNumber::ConversionFailed => Err("SCK status integer conversion failed"),
        StatusNumber::Integer(raw @ 0..=5) => Ok(raw as i32),
        StatusNumber::Integer(_) => Err("SCK status integer outside SCFrameStatus range"),
    }
}

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
pub(crate) fn read_status(
    sample: &screencapturekit::cm::CMSampleBuffer,
) -> Result<i32, &'static str> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMSampleBufferGetSampleAttachmentsArray(sample: Ref, create: u8) -> Ref;
    }
    #[link(name = "ScreenCaptureKit", kind = "framework")]
    extern "C" {
        static SCStreamFrameInfoStatus: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFNumberIsFloatType(number: Ref) -> u8;
        fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
    }
    // Every reference is borrowed from this callback's live sample. Do not
    // create attachments, retain/release them, or infer status from its pixels.
    unsafe {
        let array = CMSampleBufferGetSampleAttachmentsArray(sample.as_ptr().cast(), 0);
        if array.is_null() {
            return Err("missing SCK sample attachments array");
        }
        if CFGetTypeID(array) != CFArrayGetTypeID() {
            return Err("SCK sample attachments are not CFArray");
        }
        if CFArrayGetCount(array) < 1 {
            return Err("empty SCK sample attachments array");
        }
        let dictionary = CFArrayGetValueAtIndex(array, 0);
        if dictionary.is_null() || CFGetTypeID(dictionary) != CFDictionaryGetTypeID() {
            return Err("SCK sample attachment is not CFDictionary");
        }
        let key = SCStreamFrameInfoStatus;
        if key.is_null() || CFGetTypeID(key) != CFStringGetTypeID() {
            return Err("SCK exported status key unavailable");
        }
        let value = CFDictionaryGetValue(dictionary, key);
        let number = if value.is_null() {
            StatusNumber::Missing
        } else if CFGetTypeID(value) != CFNumberGetTypeID() {
            StatusNumber::WrongType
        } else if CFNumberIsFloatType(value) != 0 {
            StatusNumber::Floating
        } else {
            let mut raw = 0i64;
            // kCFNumberSInt64Type = 4; conversion must succeed without loss.
            if CFNumberGetValue(value, 4, (&mut raw as *mut i64).cast()) != 0 {
                StatusNumber::Integer(raw)
            } else {
                StatusNumber::ConversionFailed
            }
        };
        decode_status(number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_status_values_preserve_complete_idle_and_noncontent_states() {
        for raw in 0..=5 {
            assert_eq!(decode_status(StatusNumber::Integer(raw)), Ok(raw as i32));
        }
    }
    #[test]
    fn malformed_attachment_never_becomes_complete() {
        for value in [
            StatusNumber::Missing,
            StatusNumber::WrongType,
            StatusNumber::Floating,
            StatusNumber::ConversionFailed,
            StatusNumber::Integer(-1),
            StatusNumber::Integer(6),
            StatusNumber::Integer(i64::MIN),
            StatusNumber::Integer(i64::MAX),
        ] {
            assert!(decode_status(value).is_err(), "{value:?}");
        }
    }
}

use crate::keycodes::linux::key_from_code;
use crate::linux::keyboard::Keyboard;
use crate::rdevin::{Button, Event, EventType, KeyboardState};
use std::os::raw::{c_int, c_uchar, c_uint};
use std::time::SystemTime;
use x11::xlib;

pub const FALSE: c_int = 0;

// A global for the callbacks.
pub static mut KEYBOARD: Option<Keyboard> = None;

pub fn convert_event(code: c_uchar, type_: c_int, x: f64, y: f64) -> Option<EventType> {
    match type_ {
        xlib::KeyPress => {
            let key = key_from_code(code.into());
            Some(EventType::KeyPress(key))
        }
        xlib::KeyRelease => {
            let key = key_from_code(code.into());
            Some(EventType::KeyRelease(key))
        }
        xlib::ButtonPress => match code {
            1 => Some(EventType::ButtonPress(Button::Left)),
            2 => Some(EventType::ButtonPress(Button::Middle)),
            3 => Some(EventType::ButtonPress(Button::Right)),
            4 => Some(EventType::Wheel {
                delta_y: 1,
                delta_x: 0,
            }),
            5 => Some(EventType::Wheel {
                delta_y: -1,
                delta_x: 0,
            }),
            6 => Some(EventType::Wheel {
                delta_y: 0,
                delta_x: -1,
            }),
            7 => Some(EventType::Wheel {
                delta_y: 0,
                delta_x: 1,
            }),
            code => Some(EventType::ButtonPress(Button::Unknown(code))),
        },
        xlib::ButtonRelease => match code {
            1 => Some(EventType::ButtonRelease(Button::Left)),
            2 => Some(EventType::ButtonRelease(Button::Middle)),
            3 => Some(EventType::ButtonRelease(Button::Right)),
            4 | 5 => None,
            #[allow(clippy::useless_conversion)]
            _ => Some(EventType::ButtonRelease(Button::Unknown(code))),
        },
        xlib::MotionNotify => Some(EventType::MouseMove { x, y }),
        _ => None,
    }
}

pub fn convert(
    keyboard: &mut Option<Keyboard>,
    code: c_uint,
    type_: c_int,
    x: f64,
    y: f64,
) -> Option<Event> {
    let event_type = convert_event(code as c_uchar, type_, x, y)?;
    let kb: &mut Keyboard = (*keyboard).as_mut()?;
    let unicode = kb.add(&event_type);
    Some(Event {
        event_type,
        time: SystemTime::now(),
        unicode,
        platform_code: code as _,
        position_code: code as _,
        usb_hid: 0,
    })
}

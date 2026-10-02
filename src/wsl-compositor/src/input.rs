//! Injects control-channel input messages from the Windows host into the Wayland seat.

use bridge_protocol::ClientMessage;
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::state::State;

impl State {
    pub fn handle_control(&mut self, msg: ClientMessage) {
        match msg {
            ClientMessage::Resize { .. } => {
                // The shared framebuffer has a fixed size; the host letterboxes/scales it,
                // so no action is required here.
            }
            ClientMessage::PointerMotion { x, y } => self.on_pointer_motion(x as f64, y as f64),
            ClientMessage::PointerButton { button, pressed } => {
                self.on_pointer_button(button, pressed)
            }
            ClientMessage::PointerAxis { horizontal, vertical } => {
                self.on_pointer_axis(horizontal as f64, vertical as f64)
            }
            ClientMessage::Key { keycode, pressed } => self.on_key(keycode, pressed),
            ClientMessage::Close => self.running = false,
        }
    }

    fn on_pointer_motion(&mut self, x: f64, y: f64) {
        let pointer = match self.seat.get_pointer() {
            Some(p) => p,
            None => return,
        };
        let pos: Point<f64, _> = (x, y).into();
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.surface_under(pos);
        let time = InputTime::now();
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    fn on_pointer_button(&mut self, button: u32, pressed: bool) {
        let pointer = match self.seat.get_pointer() {
            Some(p) => p,
            None => return,
        };
        let keyboard = self.seat.get_keyboard();
        let serial = SERIAL_COUNTER.next_serial();
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };

        if pressed && !pointer.is_grabbed() {
            if let Some((window, _)) = self
                .space
                .element_under(pointer.current_location())
                .map(|(w, l)| (w.clone(), l))
            {
                self.space.raise_element(&window, true);
                if let (Some(keyboard), Some(toplevel)) = (keyboard.as_ref(), window.toplevel()) {
                    keyboard.set_focus(self, Some(toplevel.wl_surface().clone()), serial);
                }
                self.space.elements().for_each(|w| {
                    if let Some(t) = w.toplevel() {
                        t.send_pending_configure();
                    }
                });
            } else {
                self.space.elements().for_each(|w| {
                    w.set_activated(false);
                    if let Some(t) = w.toplevel() {
                        t.send_pending_configure();
                    }
                });
                if let Some(keyboard) = keyboard.as_ref() {
                    keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                }
            }
        }

        let time = InputTime::now();
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time,
            },
        );
        pointer.frame(self);
    }

    fn on_pointer_axis(&mut self, horizontal: f64, vertical: f64) {
        let pointer = match self.seat.get_pointer() {
            Some(p) => p,
            None => return,
        };
        let mut frame = AxisFrame::new(InputTime::now()).source(AxisSource::Wheel);
        if horizontal != 0.0 {
            frame = frame.value(Axis::Horizontal, horizontal);
        }
        if vertical != 0.0 {
            frame = frame.value(Axis::Vertical, vertical);
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    fn on_key(&mut self, evdev_keycode: u32, pressed: bool) {
        let keyboard = match self.seat.get_keyboard() {
            Some(k) => k,
            None => return,
        };
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::now();
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        // Backends expose keycodes as evdev + 8 (X keycode system); replicate that here.
        let keycode = Keycode::new(evdev_keycode + 8);
        keyboard.input::<(), _>(self, keycode, state, serial, time, |_, _, _| {
            FilterResult::Forward
        });
    }
}

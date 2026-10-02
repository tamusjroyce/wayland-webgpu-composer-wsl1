//! Pure input-mapping helpers: window pixel → framebuffer coordinates, and winit input
//! identifiers → Linux evdev codes.

use winit::dpi::PhysicalPosition;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Map a cursor position in physical window pixels to framebuffer pixel coordinates,
/// accounting for aspect-correct letterboxing. Returns `None` if no framebuffer size is
/// known yet or the cursor is outside the displayed image.
pub fn map_pointer(
    cursor: PhysicalPosition<f64>,
    win: (u32, u32),
    tex: (u32, u32),
) -> Option<(f32, f32)> {
    let (ww, wh) = (win.0 as f64, win.1 as f64);
    let (tw, th) = (tex.0 as f64, tex.1 as f64);
    if tw <= 0.0 || th <= 0.0 || ww <= 0.0 || wh <= 0.0 {
        return None;
    }
    let scale = (ww / tw).min(wh / th);
    let disp_w = tw * scale;
    let disp_h = th * scale;
    let off_x = (ww - disp_w) / 2.0;
    let off_y = (wh - disp_h) / 2.0;
    let fx = (cursor.x - off_x) / scale;
    let fy = (cursor.y - off_y) / scale;
    if fx < 0.0 || fy < 0.0 || fx > tw || fy > th {
        return None;
    }
    Some((fx as f32, fy as f32))
}

/// Map a winit mouse button to a Linux `BTN_*` evdev code.
pub fn mouse_button_to_evdev(button: MouseButton) -> Option<u32> {
    Some(match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::Back => 0x116,
        MouseButton::Forward => 0x115,
        MouseButton::Other(_) => return None,
    })
}

/// Map a winit physical key to a Linux evdev keycode. Covers the common keys; unmapped keys
/// return `None`.
pub fn keycode_to_evdev(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    Some(match code {
        Escape => 1,
        Digit1 => 2,
        Digit2 => 3,
        Digit3 => 4,
        Digit4 => 5,
        Digit5 => 6,
        Digit6 => 7,
        Digit7 => 8,
        Digit8 => 9,
        Digit9 => 10,
        Digit0 => 11,
        Minus => 12,
        Equal => 13,
        Backspace => 14,
        Tab => 15,
        KeyQ => 16,
        KeyW => 17,
        KeyE => 18,
        KeyR => 19,
        KeyT => 20,
        KeyY => 21,
        KeyU => 22,
        KeyI => 23,
        KeyO => 24,
        KeyP => 25,
        BracketLeft => 26,
        BracketRight => 27,
        Enter => 28,
        ControlLeft => 29,
        KeyA => 30,
        KeyS => 31,
        KeyD => 32,
        KeyF => 33,
        KeyG => 34,
        KeyH => 35,
        KeyJ => 36,
        KeyK => 37,
        KeyL => 38,
        Semicolon => 39,
        Quote => 40,
        Backquote => 41,
        ShiftLeft => 42,
        Backslash => 43,
        KeyZ => 44,
        KeyX => 45,
        KeyC => 46,
        KeyV => 47,
        KeyB => 48,
        KeyN => 49,
        KeyM => 50,
        Comma => 51,
        Period => 52,
        Slash => 53,
        ShiftRight => 54,
        NumpadMultiply => 55,
        AltLeft => 56,
        Space => 57,
        CapsLock => 58,
        F1 => 59,
        F2 => 60,
        F3 => 61,
        F4 => 62,
        F5 => 63,
        F6 => 64,
        F7 => 65,
        F8 => 66,
        F9 => 67,
        F10 => 68,
        F11 => 87,
        F12 => 88,
        ControlRight => 97,
        AltRight => 100,
        Home => 102,
        ArrowUp => 103,
        PageUp => 104,
        ArrowLeft => 105,
        ArrowRight => 106,
        End => 107,
        ArrowDown => 108,
        PageDown => 109,
        Insert => 110,
        Delete => 111,
        SuperLeft => 125,
        SuperRight => 126,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_centered_identity_when_same_size() {
        // Window exactly matches texture: no letterboxing, 1:1 mapping.
        let got = map_pointer(PhysicalPosition::new(100.0, 50.0), (200, 100), (200, 100));
        assert_eq!(got, Some((100.0, 50.0)));
    }

    #[test]
    fn pointer_accounts_for_letterbox() {
        // Window 400x100, texture 200x100 -> scale 1.0, pillarbox offset 100px on X.
        let got = map_pointer(PhysicalPosition::new(100.0, 0.0), (400, 100), (200, 100));
        assert_eq!(got, Some((0.0, 0.0)));
        let center = map_pointer(PhysicalPosition::new(200.0, 50.0), (400, 100), (200, 100));
        assert_eq!(center, Some((100.0, 50.0)));
    }

    #[test]
    fn pointer_outside_image_is_none() {
        // In the pillarbox margin.
        assert_eq!(map_pointer(PhysicalPosition::new(10.0, 50.0), (400, 100), (200, 100)), None);
    }

    #[test]
    fn pointer_none_when_no_texture() {
        assert_eq!(map_pointer(PhysicalPosition::new(1.0, 1.0), (400, 100), (0, 0)), None);
        assert_eq!(map_pointer(PhysicalPosition::new(1.0, 1.0), (0, 0), (200, 100)), None);
    }

    #[test]
    fn mouse_buttons_map() {
        assert_eq!(mouse_button_to_evdev(MouseButton::Left), Some(0x110));
        assert_eq!(mouse_button_to_evdev(MouseButton::Right), Some(0x111));
        assert_eq!(mouse_button_to_evdev(MouseButton::Middle), Some(0x112));
        assert_eq!(mouse_button_to_evdev(MouseButton::Back), Some(0x116));
        assert_eq!(mouse_button_to_evdev(MouseButton::Forward), Some(0x115));
        assert_eq!(mouse_button_to_evdev(MouseButton::Other(7)), None);
    }

    #[test]
    fn keycodes_map_common_keys() {
        assert_eq!(keycode_to_evdev(KeyCode::Escape), Some(1));
        assert_eq!(keycode_to_evdev(KeyCode::KeyA), Some(30));
        assert_eq!(keycode_to_evdev(KeyCode::Space), Some(57));
        assert_eq!(keycode_to_evdev(KeyCode::Enter), Some(28));
        assert_eq!(keycode_to_evdev(KeyCode::ArrowUp), Some(103));
        assert_eq!(keycode_to_evdev(KeyCode::F12), Some(88));
        assert_eq!(keycode_to_evdev(KeyCode::SuperLeft), Some(125));
    }

    #[test]
    fn keycodes_unmapped_return_none() {
        assert_eq!(keycode_to_evdev(KeyCode::F24), None);
        assert_eq!(keycode_to_evdev(KeyCode::PrintScreen), None);
    }

    #[test]
    fn keycodes_full_table_is_mapped() {
        use KeyCode::*;
        // Every key the host claims to support must map to the expected evdev code.
        let table: &[(KeyCode, u32)] = &[
            (Escape, 1),
            (Digit1, 2),
            (Digit2, 3),
            (Digit3, 4),
            (Digit4, 5),
            (Digit5, 6),
            (Digit6, 7),
            (Digit7, 8),
            (Digit8, 9),
            (Digit9, 10),
            (Digit0, 11),
            (Minus, 12),
            (Equal, 13),
            (Backspace, 14),
            (Tab, 15),
            (KeyQ, 16),
            (KeyW, 17),
            (KeyE, 18),
            (KeyR, 19),
            (KeyT, 20),
            (KeyY, 21),
            (KeyU, 22),
            (KeyI, 23),
            (KeyO, 24),
            (KeyP, 25),
            (BracketLeft, 26),
            (BracketRight, 27),
            (Enter, 28),
            (ControlLeft, 29),
            (KeyA, 30),
            (KeyS, 31),
            (KeyD, 32),
            (KeyF, 33),
            (KeyG, 34),
            (KeyH, 35),
            (KeyJ, 36),
            (KeyK, 37),
            (KeyL, 38),
            (Semicolon, 39),
            (Quote, 40),
            (Backquote, 41),
            (ShiftLeft, 42),
            (Backslash, 43),
            (KeyZ, 44),
            (KeyX, 45),
            (KeyC, 46),
            (KeyV, 47),
            (KeyB, 48),
            (KeyN, 49),
            (KeyM, 50),
            (Comma, 51),
            (Period, 52),
            (Slash, 53),
            (ShiftRight, 54),
            (NumpadMultiply, 55),
            (AltLeft, 56),
            (Space, 57),
            (CapsLock, 58),
            (F1, 59),
            (F2, 60),
            (F3, 61),
            (F4, 62),
            (F5, 63),
            (F6, 64),
            (F7, 65),
            (F8, 66),
            (F9, 67),
            (F10, 68),
            (F11, 87),
            (F12, 88),
            (ControlRight, 97),
            (AltRight, 100),
            (Home, 102),
            (ArrowUp, 103),
            (PageUp, 104),
            (ArrowLeft, 105),
            (ArrowRight, 106),
            (End, 107),
            (ArrowDown, 108),
            (PageDown, 109),
            (Insert, 110),
            (Delete, 111),
            (SuperLeft, 125),
            (SuperRight, 126),
        ];
        for (code, expected) in table {
            assert_eq!(keycode_to_evdev(*code), Some(*expected), "for {code:?}");
        }
    }
}

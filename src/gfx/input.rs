//! The keyboard and the mouse, as they are this frame.
//!
//! Letters and signs follow the keyboard's layout (Z is the key marked Z, on AZERTY too); the
//! digits are the keys of the top row, wherever the layout puts the digits themselves.

use std::cell::RefCell;
use std::collections::HashSet;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, NamedKey, PhysicalKey};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyCode {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Left, Right, Up, Down, PageUp, PageDown, Home, End,
    Tab, Escape, Enter, KpEnter, Backspace, Delete, Space,
    Period, Comma, Minus, Equal, LeftBracket, RightBracket,
    LeftShift, RightShift, LeftControl, RightControl, LeftAlt, RightAlt,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Default)]
struct Input {
    down: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    mouse_down: HashSet<MouseButton>,
    mouse_pressed: HashSet<MouseButton>,
    mouse_released: HashSet<MouseButton>,
    /// In points.
    mouse: (f32, f32),
    wheel: (f32, f32),
}

thread_local! {
    static INPUT: RefCell<Input> = RefCell::new(Input::default());
}

fn with<R>(f: impl FnOnce(&mut Input) -> R) -> R {
    INPUT.with(|i| f(&mut i.borrow_mut()))
}

fn letter(c: char) -> Option<KeyCode> {
    use KeyCode::*;
    Some(match c.to_ascii_lowercase() {
        'a' => A, 'b' => B, 'c' => C, 'd' => D, 'e' => E, 'f' => F, 'g' => G, 'h' => H, 'i' => I,
        'j' => J, 'k' => K, 'l' => L, 'm' => M, 'n' => N, 'o' => O, 'p' => P, 'q' => Q, 'r' => R,
        's' => S, 't' => T, 'u' => U, 'v' => V, 'w' => W, 'x' => X, 'y' => Y, 'z' => Z,
        '.' => Period, ',' => Comma, '-' => Minus, '=' => Equal, '[' => LeftBracket, ']' => RightBracket, ' ' => Space,
        _ => return None,
    })
}

fn physical(code: winit::keyboard::KeyCode) -> Option<KeyCode> {
    use winit::keyboard::KeyCode as W;
    use KeyCode::*;
    Some(match code {
        W::Digit0 => Key0, W::Digit1 => Key1, W::Digit2 => Key2, W::Digit3 => Key3, W::Digit4 => Key4,
        W::Digit5 => Key5, W::Digit6 => Key6, W::Digit7 => Key7, W::Digit8 => Key8, W::Digit9 => Key9,
        W::NumpadEnter => KpEnter,
        W::ShiftLeft => LeftShift, W::ShiftRight => RightShift,
        W::ControlLeft => LeftControl, W::ControlRight => RightControl,
        W::AltLeft => LeftAlt, W::AltRight => RightAlt,
        _ => return None,
    })
}

fn named(k: &NamedKey) -> Option<KeyCode> {
    use KeyCode::*;
    Some(match k {
        NamedKey::F1 => F1, NamedKey::F2 => F2, NamedKey::F3 => F3, NamedKey::F4 => F4,
        NamedKey::F5 => F5, NamedKey::F6 => F6, NamedKey::F7 => F7, NamedKey::F8 => F8,
        NamedKey::F9 => F9, NamedKey::F10 => F10, NamedKey::F11 => F11, NamedKey::F12 => F12,
        NamedKey::ArrowLeft => Left, NamedKey::ArrowRight => Right, NamedKey::ArrowUp => Up, NamedKey::ArrowDown => Down,
        NamedKey::PageUp => PageUp, NamedKey::PageDown => PageDown, NamedKey::Home => Home, NamedKey::End => End,
        NamedKey::Tab => Tab, NamedKey::Escape => Escape, NamedKey::Enter => Enter,
        NamedKey::Backspace => Backspace, NamedKey::Delete => Delete, NamedKey::Space => Space,
        _ => return None,
    })
}

fn key_of(e: &winit::event::KeyEvent) -> Option<KeyCode> {
    if let PhysicalKey::Code(c) = e.physical_key {
        if let Some(k) = physical(c) {
            return Some(k);
        }
    }
    match &e.logical_key {
        Key::Named(n) => named(n),
        Key::Character(s) => s.chars().next().and_then(letter),
        _ => None,
    }
}

/// Takes in what the window says; `scale` is the window's points to pixels.
pub(super) fn event(e: &WindowEvent, scale: f32) {
    with(|i| match e {
        WindowEvent::KeyboardInput { event, .. } => {
            let Some(k) = key_of(event) else { return };
            match event.state {
                ElementState::Pressed => {
                    if !event.repeat {
                        i.pressed.insert(k);
                    }
                    i.down.insert(k);
                }
                ElementState::Released => {
                    i.down.remove(&k);
                }
            }
        }
        WindowEvent::CursorMoved { position, .. } => i.mouse = (position.x as f32 / scale, position.y as f32 / scale),
        WindowEvent::MouseInput { state, button, .. } => {
            let b = match button {
                winit::event::MouseButton::Left => MouseButton::Left,
                winit::event::MouseButton::Right => MouseButton::Right,
                winit::event::MouseButton::Middle => MouseButton::Middle,
                _ => return,
            };
            match state {
                ElementState::Pressed => {
                    i.mouse_down.insert(b);
                    i.mouse_pressed.insert(b);
                }
                ElementState::Released => {
                    i.mouse_down.remove(&b);
                    i.mouse_released.insert(b);
                }
            }
        }
        WindowEvent::MouseWheel { delta, .. } => {
            let (x, y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 40.0, p.y as f32 / 40.0),
            };
            i.wheel.0 += x;
            i.wheel.1 += y;
        }
        // Keys let go while the window was elsewhere would stay down for ever.
        WindowEvent::Focused(false) => {
            i.down.clear();
            i.mouse_down.clear();
        }
        _ => {}
    });
}

/// What happened once is forgotten at the end of the frame.
pub(super) fn end_frame() {
    with(|i| {
        i.pressed.clear();
        i.mouse_pressed.clear();
        i.mouse_released.clear();
        i.wheel = (0.0, 0.0);
    });
}

pub fn is_key_down(k: KeyCode) -> bool {
    with(|i| i.down.contains(&k))
}

pub fn is_key_pressed(k: KeyCode) -> bool {
    with(|i| i.pressed.contains(&k))
}

pub fn get_keys_pressed() -> HashSet<KeyCode> {
    with(|i| i.pressed.clone())
}

pub fn is_mouse_button_down(b: MouseButton) -> bool {
    with(|i| i.mouse_down.contains(&b))
}

pub fn is_mouse_button_pressed(b: MouseButton) -> bool {
    with(|i| i.mouse_pressed.contains(&b))
}

pub fn is_mouse_button_released(b: MouseButton) -> bool {
    with(|i| i.mouse_released.contains(&b))
}

pub fn mouse_position() -> (f32, f32) {
    with(|i| i.mouse)
}

/// The wheel this frame: up is positive.
pub fn mouse_wheel() -> (f32, f32) {
    with(|i| i.wheel)
}

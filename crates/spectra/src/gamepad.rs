//! Gamepads, as the same remote control the keyboard is.
//!
//! Buttons go by position, so the bottom face button is "select" whether it
//! says A or Cross. What is printed on them only matters for prompts, so
//! each press says which kind of pad it came from. gilrs reads evdev on
//! Linux and IOKit on macOS; it runs on its own thread and blocks between
//! events, so an idle pad costs nothing.

use gilrs::{Axis, Button, EventType, Gilrs};
use iced::Subscription;
use iced::futures::SinkExt;

use crate::Remote;
use crate::ui::Pad;

/// How far a stick has to lean to count as a press.
const STICK_THRESHOLD: f32 = 0.6;

pub fn subscription() -> Subscription<(Pad, Remote)> {
    Subscription::run(|| {
        iced::stream::channel(16, async |mut output| {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || {
                let Ok(mut gilrs) = Gilrs::new() else { return };
                let (mut held_x, mut held_y) = (false, false);
                loop {
                    let Some(event) = gilrs.next_event_blocking(None) else {
                        continue;
                    };
                    // A stick is a d-pad here: one step per lean, not a
                    // stream of them.
                    let lean = |held: &mut bool, value: f32, minus, plus| {
                        let leaning = value.abs() > STICK_THRESHOLD;
                        let fire = leaning && !*held;
                        *held = leaning;
                        fire.then_some(if value > 0.0 { plus } else { minus })
                    };
                    let remote = match event.event {
                        EventType::ButtonPressed(button, _) => match button {
                            Button::DPadUp => Some(Remote::Up),
                            Button::DPadDown => Some(Remote::Down),
                            Button::DPadLeft => Some(Remote::Left),
                            Button::DPadRight => Some(Remote::Right),
                            Button::South => Some(Remote::Select),
                            Button::East => Some(Remote::Back),
                            Button::North => Some(Remote::Keep),
                            Button::West => Some(Remote::Favorite),
                            // The pad's own home button: the start menu.
                            Button::Mode => Some(Remote::Settings),
                            Button::Start => Some(Remote::PlayPause),
                            Button::Select => Some(Remote::Menu),
                            Button::LeftTrigger => Some(Remote::Previous),
                            Button::RightTrigger => Some(Remote::Next),
                            _ => None,
                        },
                        EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                            lean(&mut held_y, value, Remote::Down, Remote::Up)
                        }
                        EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                            lean(&mut held_x, value, Remote::Left, Remote::Right)
                        }
                        _ => None,
                    };
                    let Some(remote) = remote else { continue };
                    let pad = gilrs.gamepad(event.id);
                    let pad = Pad::identify(pad.vendor_id(), pad.product_id(), pad.name());
                    if tx.unbounded_send((pad, remote)).is_err() {
                        return;
                    }
                }
            });
            use iced::futures::StreamExt;
            while let Some(press) = rx.next().await {
                if output.send(press).await.is_err() {
                    break;
                }
            }
        })
    })
}

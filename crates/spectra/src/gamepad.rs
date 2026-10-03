//! Gamepads, as the same remote control the keyboard is.
//!
//! Buttons go by position, so the bottom face button is "select" whether it
//! says A or Cross. gilrs reads evdev on Linux and IOKit on macOS; it runs on
//! its own thread and blocks between events, so an idle pad costs nothing.

use gilrs::{Axis, Button, EventType, Gilrs};
use iced::Subscription;
use iced::futures::SinkExt;

use crate::Remote;

/// How far a stick has to lean to count as a press.
const STICK_THRESHOLD: f32 = 0.6;

pub fn subscription() -> Subscription<Remote> {
    Subscription::run(|| {
        iced::stream::channel(16, async |mut output| {
            let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || {
                let Ok(mut gilrs) = Gilrs::new() else { return };
                let mut stick_held = false;
                loop {
                    let Some(event) = gilrs.next_event_blocking(None) else {
                        continue;
                    };
                    let remote = match event.event {
                        EventType::ButtonPressed(button, _) => match button {
                            Button::DPadUp => Some(Remote::Up),
                            Button::DPadDown => Some(Remote::Down),
                            Button::South => Some(Remote::Select),
                            Button::East => Some(Remote::Back),
                            Button::North => Some(Remote::Keep),
                            Button::Start => Some(Remote::PlayPause),
                            Button::Select => Some(Remote::Library),
                            Button::LeftTrigger => Some(Remote::Previous),
                            Button::RightTrigger => Some(Remote::Next),
                            _ => None,
                        },
                        // A stick is a d-pad here: one step per lean, not a
                        // stream of them.
                        EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                            let leaning = value.abs() > STICK_THRESHOLD;
                            let fire = leaning && !stick_held;
                            stick_held = leaning;
                            fire.then_some(if value > 0.0 {
                                Remote::Up
                            } else {
                                Remote::Down
                            })
                        }
                        _ => None,
                    };
                    if let Some(remote) = remote
                        && tx.unbounded_send(remote).is_err()
                    {
                        return;
                    }
                }
            });
            use iced::futures::StreamExt;
            while let Some(remote) = rx.next().await {
                if output.send(remote).await.is_err() {
                    break;
                }
            }
        })
    })
}

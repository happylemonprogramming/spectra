//! The card the TV shows between films and games: Spectra's name on its
//! dark gradient, so the TV is never the bare desktop. It is its own small
//! window, `spectra --tv-card`, started by the window that has the TV and
//! ended with it; Hyprland puts it on the TV's screen, full screen, and VLC
//! and RetroArch open over it.

use iced::gradient::Linear;
use iced::widget::{column, container, text};
use iced::{Background, Color, Element, Fill, Radians, Size, Task, Theme, window};

use crate::FONT;

pub fn run() -> iced::Result {
    iced::application(
        || ((), Task::none()),
        |_: &mut (), _: ()| Task::none(),
        view,
    )
    .title("Spectra on the TV")
    .theme(|_: &()| Theme::Dark)
    .default_font(FONT)
    .window(window::Settings {
        size: Size::new(960.0, 540.0),
        // Not `spectra`, so launchers and window rules do not take it for
        // the window itself.
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "spectra-tv-card".into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
}

fn view(_: &()) -> Element<'_, ()> {
    let words = column![
        text("Spectra")
            .size(72)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.92)),
        text("Pick something to play on the computer")
            .size(24)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.5)),
    ]
    .spacing(12)
    .align_x(iced::Center);
    container(words)
        .width(Fill)
        .height(Fill)
        .center(Fill)
        .style(|_| container::Style {
            background: Some(Background::Gradient(
                Linear::new(Radians(std::f32::consts::FRAC_PI_4))
                    .add_stop(0.0, Color::from_rgb8(42, 23, 69))
                    .add_stop(1.0, Color::from_rgb8(11, 11, 20))
                    .into(),
            )),
            ..Default::default()
        })
        .into()
}

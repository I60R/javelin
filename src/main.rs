extern crate libc;

use clap::{Parser, AppSettings};
use input::{
    event::{EventTrait, pointer::PointerEventTrait, PointerEvent as __},
    Libinput,
    LibinputInterface
};
use swayipc::{Rect, Node};
use std::{
    path::Path,
    fs::{File, OpenOptions},
    os::unix::{
        fs::OpenOptionsExt,
        io::{RawFd, FromRawFd, IntoRawFd}
    },
    thread
};

// Interface implementation was copied from Smithay/input event loop example
// https://github.com/Smithay/input.rs/tree/1d83b2e868bc408f272c0df3cd9ac2a4#usage
struct Interface;
impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<RawFd, i32> {
        use libc::{O_RDONLY, O_RDWR, O_WRONLY};
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_RDONLY != 0) | (flags & O_RDWR != 0))
            .write((flags & O_WRONLY != 0) | (flags & O_RDWR != 0))
            .open(path)
            .map(|f| f.into_raw_fd())
            .map_err(|e| e.raw_os_error().unwrap())
    }

    fn close_restricted(&mut self, fd: RawFd) {
        unsafe {
            File::from_raw_fd(fd);
        }
    }
}

fn main() {
    let mut conn = swayipc::Connection::new().expect("Cannot connect to Sway!");

    let Args {
        touchpad_device,
        trackpoint_device,
        pointer_acceleration,
        javelin_acceleration,
        pointer_cooldown,
        javelin_cooldown,
        reload_msec,
        tremble_msec,
        x_split_reload,
        y_split_reload,
        offsets
    } = Args::parse();

    let offsets: std::collections::HashMap<_, _> = offsets.iter()
        .map(|s| {
            let mut it = s.split(':');
            let app = it.next().unwrap();
            let offsets = (
                it.next().unwrap_or("0").parse::<i32>().expect("Invalid x offset format"),
                it.next().unwrap_or("0").parse::<i32>().expect("Invalid y offset format"),
            );
            (app, offsets)
        })
        .collect();

    conn.run_command(format!("
        focus_follows_mouse always
        mouse_warping container
        seat * hide_cursor {reload_msec}
        input type:touchpad pointer_accel {javelin_acceleration}
    ")).unwrap();

    let mut libinput = Libinput::new_from_path(Interface);

    let touchpad = libinput.path_add_device(&touchpad_device)
        .expect("Cannot open touchpad device");

    if let None = libinput.path_add_device(&trackpoint_device) {
        eprintln!("Cannot open trackpoint device {trackpoint_device}; SKIP")
    }

    let mut past_event_time = 0;
    let mut javelin = true;

    let mut past_tremble_time = 0;
    let mut tremble = true;
    let mut trembles = [
        (0, 15), (15, 0), (0, -15), (-15, 0), (0, 0),
        (0, -15), (-15, 0), (0, 15), (15, 0), (0, 0),
        (-15, 0), (0, 15), (15, 0), (0, -15), (0, 0),
        (15, 0), (0, -15), (-15, 0), (0, 15), (0, 0),
    ]
        .into_iter()
        .cycle();

    loop {
        libinput.dispatch().unwrap();

        let event = match libinput.next() {
            Some(input::Event::Pointer(ev)) => ev,
            _ => {
                if tremble {
                    thread::sleep(std::time::Duration::from_millis(tremble_msec as u64));

                    let (x, y) = trembles.next().unwrap();
                    if (x, y) == (0, 0) {
                        tremble = false;
                        continue
                    }

                    conn.run_command(format!("seat seat0 cursor move {x} {y}")).unwrap();
                }
                continue
            },
        };

        if let motion_event @ (__::Motion(_) | __::MotionAbsolute(_)) = event {

            let current_event_time = motion_event.time();
            let delta_time;

            (past_event_time, delta_time) =
                (current_event_time, current_event_time - past_event_time);

            if motion_event.device() != touchpad {
                past_event_time = past_event_time.saturating_sub(pointer_cooldown);
                javelin = false;
                continue
            }

            if delta_time > reload_msec {
                conn.run_command(format!("
                    input type:touchpad pointer_accel {javelin_acceleration}
                ")).unwrap();

                libinput.dispatch().unwrap();

                let focused_window = conn.get_tree().unwrap()
                    .find_focused(|n| n.nodes.is_empty()).unwrap();
                let Rect { mut x, mut y, width, height, .. } = focused_window.rect;
                (x, y) = (
                    x + width / x_split_reload,
                    y + height / y_split_reload
                );
                let Node { app_id, window_properties, .. } = focused_window;
                let focused_application = app_id
                    .or_else(|| window_properties
                        .and_then(|p| p.instance.or(p.class).or(p.title)));
                if let Some((x_offset, y_offset)) = offsets
                    .get(focused_application.as_deref().unwrap_or("none"))
                {
                    x += x_offset;
                    y += y_offset;
                }

                libinput.dispatch().unwrap();

                conn.run_command(format!("
                    seat seat0 cursor set {x} {y}
                ")).unwrap();

                javelin = true;
                tremble = true;
                continue
            }

            if javelin && delta_time > javelin_cooldown {
                conn.run_command(format!("
                    input type:touchpad pointer_accel {pointer_acceleration}
                ")).unwrap();

                javelin = false;
                continue
            }

            if delta_time > pointer_cooldown {
                conn.run_command(format!("
                    input type:touchpad pointer_accel {javelin_acceleration}
                ")).unwrap();

                javelin = true;
                tremble = true;
                continue
            }

            if javelin {
                let delta_time = current_event_time - past_tremble_time;
                tremble = true;

                if delta_time < tremble_msec {
                    continue
                }

                past_tremble_time = current_event_time;

                let (mut x, mut y) = trembles.next().unwrap();
                if (x, y) == (0, 0) {
                    (x, y) = trembles.next().unwrap();
                }

                conn.run_command(format!("seat seat0 cursor move {x} {y}")).unwrap();
            }

        } else if let scroll_event @ (
            __::ScrollContinuous(_) | __::ScrollFinger(_) | __::ScrollWheel(_)
        ) = event {
            past_event_time = scroll_event.time();
            javelin = false
        }
    }
}

#[derive(Parser)]
#[clap(author, version, global_setting = AppSettings::DeriveDisplayOrder)]
struct Args {
    #[clap(display_order=0, long, default_value = "/dev/input/event16")]
    touchpad_device: String,

    #[clap(display_order=0, long, default_value = "/dev/input/event15")]
    trackpoint_device: String,

    #[clap(display_order=0, long, default_value = "-0.2")]
    pointer_acceleration: f32,

    #[clap(display_order=0, long, default_value = "0.9")]
    javelin_acceleration: f32,

    #[clap(display_order=0, long, default_value = "400")]
    pointer_cooldown: u32,

    #[clap(display_order=0, long, default_value = "60")]
    javelin_cooldown: u32,

    #[clap(display_order=0, long, default_value = "4096")]
    reload_msec: u32,

    #[clap(display_order=0, long, default_value = "32")]
    tremble_msec: u32,

    #[clap(display_order=0, long, default_value = "2")]
    x_split_reload: i32,

    #[clap(display_order=0, long, default_value = "2")]
    y_split_reload: i32,

    /// app_id:x:y
    offsets: Vec<String>
}

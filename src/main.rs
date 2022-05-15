extern crate libc;

use clap::{Parser, AppSettings};
use input::{event::{EventTrait, pointer::PointerEventTrait, PointerEvent as __}, Libinput, LibinputInterface};
use swayipc::{Rect, Node};
use std::{
    path::Path,
    fs::{File, OpenOptions},
    os::unix::{
        fs::OpenOptionsExt,
        io::{RawFd, FromRawFd, IntoRawFd}
    }
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
        touchpad_device, trackpoint_device,
        touchpad_pointer_accel, trackpoint_pointer_accel,
        touchpad_scroll_factor, trackpoint_scroll_factor,
        x_ratio, y_ratio,
        reset_msec, trackpoint_withdraw_msec,
        precision_factor, precision_mode_msec, precision_mode_withdraw_msec,
        offsets,
        scroll_deposit_msec
    } = Args::parse();
    let offsets: std::collections::HashMap<_, _> = offsets.iter()
        .map(|s| {
            let mut it = s.split(':');
            let app = it.next().unwrap();
            let x_offset = it.next().unwrap_or("0").parse::<i32>().expect("Invalid offset format");
            let y_offset = it.next().unwrap_or("0").parse::<i32>().expect("Invalid offset format");
            (app, (x_offset, y_offset))
        })
        .collect();
    conn.run_command(format!("
        focus_follows_mouse always
        mouse_warping container
        input type:touchpad pointer_accel {touchpad_pointer_accel}
        input type:trackpoint scroll_factor {trackpoint_scroll_factor}
        input type:trackpoint pointer_accel {trackpoint_pointer_accel}
    ")).unwrap();
    let mut input = Libinput::new_from_path(Interface);
    let input_raw = std::ptr::addr_of_mut!(input); // break uniqueness rules to remove cursor lag
    let (_trackpoint, touchpad) = (
        input.path_add_device(&trackpoint_device).expect("Cannot open trackpoint device: this program is unusable with touchpad only"),
        input.path_add_device(&touchpad_device).expect("Cannot open touchpad device"),
    );
    let mut past_event_time = 0;
    loop {
        unsafe {(*input_raw).dispatch().unwrap()}
        for event in &mut input {
            if let input::Event::Pointer(motion_event @ (__::Motion(_) | __::MotionAbsolute(_))) = event {
                let current_event_time = motion_event.time();
                let delta_time;
                (past_event_time, delta_time) = (current_event_time, current_event_time - past_event_time);
                if motion_event.device() == touchpad {
                    if delta_time > reset_msec {
                        conn.run_command(format!("
                            input type:touchpad pointer_accel {touchpad_pointer_accel}
                        ")).unwrap();
                        unsafe {(*input_raw).dispatch().unwrap()}
                        let focused_window = conn.get_tree().unwrap().find_focused(|n| n.nodes.len() == 0).unwrap();
                        unsafe {(*input_raw).dispatch().unwrap()}
                        let Rect { mut x, mut y, width, height, .. } = focused_window.rect;
                        (x, y) = (
                            x + width / x_ratio,
                            y + height / y_ratio
                        );
                        let Node {app_id, window_properties, .. } = focused_window;
                        let app = app_id.or(window_properties.and_then(|p| p.instance.or(p.class).or(p.title)));
                        if let Some((x_offset, y_offset)) = offsets.get(app.as_deref().unwrap_or("none")) {
                            x += x_offset;
                            y += y_offset;
                        }
                        conn.run_command(format!("seat seat0 cursor set {x} {y}")).unwrap();
                        unsafe {(*input_raw).dispatch().unwrap()}
                    } else if delta_time > precision_mode_msec {
                        let touchpad_scroll_factor_precise = touchpad_scroll_factor / precision_factor;
                        let touchpad_pointer_accel_precise = touchpad_pointer_accel / precision_factor;
                        conn.run_command(format!("
                            input type:touchpad scroll_factor {touchpad_scroll_factor_precise}
                            input type:touchpad pointer_accel {touchpad_pointer_accel_precise}
                        ")).unwrap();
                        past_event_time -= precision_mode_withdraw_msec;
                    }
                } else {
                    past_event_time -= trackpoint_withdraw_msec // always reset touchpad after trackpoint use
                }
            } else if let input::Event::Pointer(scroll_event @ (__::ScrollContinuous(_) | __::ScrollFinger(_) | __::ScrollWheel(_))) = event {
                past_event_time = scroll_event.time()
            }
        }
    }
}

#[derive(Parser)]
#[clap(author, version, global_setting = AppSettings::DeriveDisplayOrder)]
struct Args {
    #[clap(display_order=0, long, default_value = "/dev/input/event16")]
    touchpad_device: String,

    #[clap(display_order=0, long, default_value = "/dev/input/event14")]
    trackpoint_device: String,

    #[clap(display_order=0, long, default_value = "1.0")]
    touchpad_pointer_accel: f32,

    #[clap(display_order=0, long, default_value = "5.0")]
    touchpad_scroll_factor: f32,

    #[clap(display_order=0, long, default_value = "0")]
    trackpoint_pointer_accel: f32,

    #[clap(display_order=0, long, default_value = "0")]
    trackpoint_scroll_factor: f32,

    #[clap(display_order=0, long, default_value = "2")]
    x_ratio: i32,

    #[clap(display_order=0, long, default_value = "2")]
    y_ratio: i32,

    #[clap(display_order=0, long, default_value = "500")]
    reset_msec: u32,

    #[clap(display_order=0, long, default_value = "100")]
    precision_mode_msec: u32,

    #[clap(display_order=0, long, default_value = "300")]
    precision_mode_withdraw_msec: u32,

    #[clap(display_order=0, long, default_value = "5")]
    precision_factor: f32,

    #[clap(display_order=0, long, default_value = "470")]
    trackpoint_withdraw_msec: u32,

    #[clap(display_order=0, long, default_value = "20")]
    scroll_deposit_msec: u32,


    /// app_id:x:y
    offsets: Vec<String>
}
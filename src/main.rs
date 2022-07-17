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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    let terminate = Arc::new(AtomicBool::default());

    for sig in signal_hook::consts::TERM_SIGNALS {

        let sig_handler = Arc::clone(&terminate);
        signal_hook::flag::register(*sig, sig_handler)
            .unwrap();
    }


    let mut conn = swayipc::Connection::new()
        .expect("Cannot connect to Sway!");


    let Args {
        device,
        r#type,
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
            let app = it.next()
                .unwrap();
            let offsets = (
                it.next()
                    .unwrap_or("0")
                    .parse::<i32>()
                    .expect("Invalid x offset format"),
                it.next()
                    .unwrap_or("0")
                    .parse::<i32>()
                    .expect("Invalid y offset format"),
            );
            (app, offsets)
        })
        .collect();

    let r#type = r#type
        .unwrap_or("touchpad".to_string());


    let mut libinput = Libinput::new_from_path(Interface);


    let device_path = device.unwrap_or_else(|| {

        let mut event_mouse_devices = vec![];

        for entry in
            std::fs::read_dir("/dev/input/by-path")
                .expect("Cannot inspect /dev/input/by-path directory")
                .map(|p| p
                    .expect("Cannot inspect some input device")
                )
        {
            let device_path = entry
                .path()
                .to_string_lossy()
                .to_string();

            if device_path.contains("event-mouse") {
                event_mouse_devices
                    .push(device_path)
            }
        }

        event_mouse_devices
            .sort();

        println!("The following devices were found: {event_mouse_devices:#?}");

        if event_mouse_devices.len() < 2 {
            eprintln!("Javelin could be annoying with only one pointer device available")
        }

        event_mouse_devices
            .pop()
            .expect("No pointer devices were found")
    });

    let pointer_device = libinput
        .path_add_device(&device_path)
        .expect("Cannot open pointer device");


    conn.run_command(format!("
        focus_follows_mouse always
        mouse_warping container
        seat * hide_cursor {reload_msec}
        input type:{type} pointer_accel {javelin_acceleration}
    "))
    .unwrap();


    let mut past_event_time = 0;
    let mut javelin = true;

    let mut past_tremble_time = 0;
    let mut tremble = true;
    let mut trembles = [
        15, 8, 10, 9, 16, 7, 18, 13, 6, 11, 19, 12, 17
    ]
        .into_iter()
        .scan((0..8).cycle(), |dir, dist| {
            match dir.next().unwrap() {
                0 => Some([(0, dist), (dist, 0), (0, -dist), (-dist, 0), (0, 0)]),
                1 => Some([(-dist, 0), (0, -dist), (dist, 0), (0, dist), (0, 0)]),
                2 => Some([(0, dist), (-dist, 0), (0, -dist), (dist, 0), (0, 0)]),
                3 => Some([(dist, 0), (0, -dist), (-dist, 0), (0, dist), (0, 0)]),
                4 => Some([(dist, 0), (0, dist), (-dist, 0), (0, -dist), (0, 0)]),
                5 => Some([(0, -dist), (-dist, 0), (0, dist), (dist, 0), (0, 0)]),
                6 => Some([(-dist, 0), (0, dist), (dist, 0), (0, -dist), (0, 0)]),
                7 => Some([(0, -dist), (dist, 0), (0, dist), (-dist, 0), (0, 0)]),
                _ => unreachable!()
            }
        })
        .flat_map(|x| x)
        .cycle();


    loop {
        libinput
            .dispatch()
            .unwrap();

        let event = match libinput.next() {
            Some(input::Event::Pointer(ev)) => ev,
            _ => {
                if tremble {

                    let mut wait_times = tremble_msec / (tremble_msec / 6);

                    while wait_times > 0 {

                        libinput
                            .dispatch()
                            .unwrap();
                        spin_sleep::sleep(std::time::Duration::from_millis(6 as u64));

                        wait_times -= 1;
                    }

                    let (x, y) = trembles.next()
                        .unwrap();
                    if (x, y) == (0, 0) {
                        tremble = false;
                    } else {
                        conn.run_command(format!("seat seat0 cursor move {x} {y}"))
                            .unwrap();
                    }
                }

                if terminate.load(Ordering::Relaxed) {

                    conn.run_command(format!("
                        input type:{type} pointer_accel 0
                        seat * hide_cursor 0
                    "))
                    .unwrap();

                    return
                }

                continue
            },
        };

        match event {
            __::Motion(_) | __::MotionAbsolute(_) => {

                let current_event_time = event.time();
                let delta_time;

                (past_event_time, delta_time) =
                    (current_event_time, current_event_time.saturating_sub(past_event_time));

                if event.device() != pointer_device {
                    past_event_time = past_event_time.saturating_sub(pointer_cooldown);
                    javelin = false;

                    continue
                }

                if delta_time > reload_msec {
                    conn.run_command(format!("
                        input type:{type} pointer_accel {javelin_acceleration}
                    "))
                    .unwrap();

                    libinput
                        .dispatch()
                        .unwrap();

                    let focused_window = conn
                        .get_tree()
                        .unwrap()
                        .find_focused(|n| n.nodes.is_empty())
                        .unwrap();

                    let Rect { mut x, mut y, width, height, .. } = focused_window.rect;
                    (x, y) = (
                        x + width / x_split_reload,
                        y + height / y_split_reload
                    );
                    let Node { app_id, window_properties, .. } = focused_window;

                    let focused_application = app_id
                        .or_else(|| window_properties
                            .and_then(|p| p.instance
                                .or(p.class)
                                .or(p.title)));
                    if let Some((x_offset, y_offset)) = offsets
                        .get(focused_application
                            .as_deref()
                            .unwrap_or("none")
                        )
                    {
                        x += x_offset;
                        y += y_offset;
                    }

                    libinput
                        .dispatch()
                        .unwrap();

                    conn.run_command(format!("
                        seat seat0 cursor set {x} {y}
                    "))
                    .unwrap();

                    javelin = true;
                    tremble = true;

                    continue
                }

                if javelin && delta_time > javelin_cooldown {
                    conn.run_command(format!("
                        input type:{type} pointer_accel {pointer_acceleration}
                    "))
                    .unwrap();

                    javelin = false;

                    continue
                }

                if delta_time > pointer_cooldown {
                    conn.run_command(format!("
                        input type:{type} pointer_accel {javelin_acceleration}
                    "))
                    .unwrap();

                    javelin = true;
                    tremble = true;

                    continue
                }

                if javelin {
                    let delta_time = current_event_time.saturating_sub(past_tremble_time);
                    tremble = true;

                    if delta_time < tremble_msec {
                        continue
                    }

                    past_tremble_time = current_event_time;

                    let (mut x, mut y) = trembles.next()
                        .unwrap();
                    if (x, y) == (0, 0) {
                        (x, y) = trembles.next()
                            .unwrap();
                    }

                    conn.run_command(format!("seat seat0 cursor move {x} {y}"))
                        .unwrap();
                }
            },

            __::ScrollContinuous(_) | __::ScrollFinger(_) | __::ScrollWheel(_) => {

                past_event_time = event.time();

                javelin = false;
            },

            _ => {}
        }
    }
}


#[derive(Parser)]
#[clap(author, version, global_setting = AppSettings::DeriveDisplayOrder)]
struct Args {
    #[clap(display_order=0, long, requires = "type")]
    device: Option<String>,

    #[clap(display_order=0, long)]
    r#type: Option<String>,

    #[clap(display_order=0, long, default_value = "-0.2")]
    pointer_acceleration: f32,

    #[clap(display_order=0, long, default_value = "0.8")]
    javelin_acceleration: f32,

    #[clap(display_order=0, long, default_value = "400")]
    pointer_cooldown: u32,

    #[clap(display_order=0, long, default_value = "32")]
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

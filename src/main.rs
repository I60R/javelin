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
    },
    error::Error,
    time::Duration,
    collections::HashMap
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


macro_rules! dispatch {
    ($libinput: ident) => {
        $libinput
            .dispatch()
            .unwrap()
    };
}


fn main() {
    let terminate = Arc::new(AtomicBool::default());

    main::register_signal_handling(&terminate);

    let conn = swayipc::Connection::new()
        .expect("Cannot connect to Sway!");

    let mut libinput = Libinput::new_from_path(Interface);

    let args = main::get_arguments();

    let pointer_device = libinput
        .path_add_device(&args.device_path)
        .expect("Cannot open pointer device");

    handle_events(conn, libinput, terminate, pointer_device, args)
        .unwrap()
}

mod main {
    use super::*;

    pub fn register_signal_handling(terminate: &Arc<AtomicBool>) {
        for sig in signal_hook::consts::TERM_SIGNALS {

            let sig_handler = Arc::clone(terminate);
            signal_hook::flag::register(*sig, sig_handler)
                .expect("Cannot register signal handler");
        }
    }

    pub fn get_arguments() -> ArgContext {
        let args = Args::parse();
        println!("{args:#?}");

        let offsets = args.offsets.iter()
            .map(parse_offset_value)
            .collect();

        let device_type = args.device_type
            .clone()
            .unwrap_or("touchpad".to_string());

        let device_path = args.device
            .clone()
            .unwrap_or_else(detect_touchpad_device);

        ArgContext {
            args,
            offsets,
            device_path,
            device_type,
        }
    }
}


fn handle_events(
    mut conn: swayipc::Connection,
    mut libinput: Libinput,
    terminate: Arc<AtomicBool>,
    pointer_device: input::Device,
    args: ArgContext,
) -> Result<(), Box<dyn Error>> {

    let ArgContext {
        offsets,
        device_type,
        args: Args {
            pointer_acceleration,
            javelin_acceleration,
            pointer_cooldown,
            javelin_cooldown,
            reload_msec,
            tremble_msec,
            x_split_reload,
            y_split_reload,
            ..
        },
        ..
    } = args;

    conn.run_command(format!("
        focus_follows_mouse always
        mouse_warping container
        seat * hide_cursor {reload_msec}
        input type:{device_type} pointer_accel {javelin_acceleration}
    "))?;

    let mut past_event_time = 0;
    let mut javelin = true;

    let mut past_tremble_time = 0;
    let mut trembling = true;
    let mut trembles = trembles();

    let mut tremble = || trembles
        .next()
        .unwrap();


    loop {
        dispatch!(libinput);

        let Some(input::Event::Pointer(event))
            = libinput.next() else {

            if trembling {

                let mut wait_times = tremble_msec / (tremble_msec / 6);

                while wait_times > 0 {

                    dispatch!(libinput);

                    spin_sleep::sleep(Duration::from_millis(6));

                    wait_times -= 1;
                }

                let (x, y) = tremble();
                if (x, y) == (0, 0) {
                    trembling = false;
                } else {
                    conn.run_command(format!("seat seat0 cursor move {x} {y}"))?;
                }
            } else {
                spin_sleep::sleep(Duration::from_millis(16));
            }

            if terminate.load(Ordering::Relaxed) {

                println!("\nGraceful shutdown");

                conn.run_command(format!("
                    input type:{device_type} pointer_accel 0
                    seat * hide_cursor 0
                "))?;

                std::process::exit(0)
            }

            continue
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
                        input type:{device_type} pointer_accel {javelin_acceleration}
                    "))?;

                    dispatch!(libinput);

                    let focused_window = conn
                        .get_tree()?
                        .find_focused(|n| n.nodes.is_empty())
                        .expect("Cannot get focused container");

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

                    dispatch!(libinput);

                    conn.run_command(format!("
                        seat seat0 cursor set {x} {y}
                    "))?;

                    javelin = true;
                    trembling = true;

                    continue
                }

                if javelin && delta_time > javelin_cooldown {
                    conn.run_command(format!("
                        input type:{device_type} pointer_accel {pointer_acceleration}
                    "))?;

                    javelin = false;

                    continue
                }

                if delta_time > pointer_cooldown {
                    conn.run_command(format!("
                        input type:{device_type} pointer_accel {javelin_acceleration}
                    "))?;

                    javelin = true;
                    trembling = true;

                    continue
                }

                if javelin {
                    let delta_time = current_event_time.saturating_sub(past_tremble_time);
                    trembling = true;

                    if delta_time < tremble_msec {
                        continue
                    }

                    past_tremble_time = current_event_time;

                    let (mut x, mut y) = tremble();
                    if (x, y) == (0, 0) {
                        (x, y) = tremble();
                    }

                    conn.run_command(format!("seat seat0 cursor move {x} {y}"))?;
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


fn trembles() -> impl Iterator<Item = (i32, i32)> {
    [ 15, 8, 10, 9, 16, 7, 18, 13, 6, 11, 19, 12, 17 ]
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
        .cycle()
}


fn parse_offset_value(arg: &String) -> (String, (i32, i32)) {
    let mut app_x_y = arg.split(':');

    let app = app_x_y.next()
        .unwrap()
        .to_string();

    let offsets = (
        app_x_y.next()
            .unwrap_or("0")
            .parse::<i32>()
            .expect("Invalid x offset format"),
        app_x_y.next()
            .unwrap_or("0")
            .parse::<i32>()
            .expect("Invalid y offset format"),
    );

    (app, offsets)
}


fn detect_touchpad_device() -> String {
    let list_devices_output = std::process::Command::new("libinput")
        .arg("list-devices")
        .output()
        .expect("Error executing `libinput list-devices`");

    if !list_devices_output.status.success() {
        panic!("Error executing `libinput list-devices`: {list_devices_output:#?}")
    }

    let libinput_devices = String::from_utf8(list_devices_output.stdout)
        .expect("Invalid `libinput list-devices` output");

    for dev_descr in libinput_devices
        .split("\n\n")
    {
        let mut descr_lines = dev_descr
            .split('\n');

        let dev_name = descr_lines
            .next()
            .expect("Cannot get device name")
            .to_ascii_lowercase();

        if dev_name.contains("touchpad") ||
            dev_name.contains("touch pad")
        {
            let dev_path = descr_lines
                .next()
                .expect("Cannot get device path")
                .split(":")
                .nth(1)
                .expect("Invalid device path line")
                .trim()
                .to_string();

            println!("\n[Use touchpad device]\n{dev_descr}\n");

            return dev_path
        }
    }

    panic!("No touchpad device detected: specify it through --device flag")
}


pub struct ArgContext {
    args: Args,
    device_type: String,
    device_path: String,
    offsets: HashMap<String, (i32, i32)>
}


#[derive(Parser, Debug)]
#[clap(author, version, global_setting = AppSettings::DeriveDisplayOrder)]
struct Args {
    #[clap(display_order=0, long, requires = "device-type")]
    device: Option<String>,

    #[clap(display_order=0, long)]
    device_type: Option<String>,

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

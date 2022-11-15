extern crate libc;

use input::{
    event::{
        EventTrait as _,
        pointer::PointerEventTrait,
        PointerEvent as __
    },
    Libinput,
    LibinputInterface
};
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
};


// This struct implementation was copied from Smithay/input event loop example
// https://github.com/Smithay/input.rs/tree/1d83b2e868bc408f272c0df3cd9ac2a4#usage
//
// Used to iterate over touchpad input events.
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


// Macro to reduce a common chunk of code in event loop.
//
// Dispatches libinput events in order to reduce pointer lags.
macro_rules! dispatch {
    ($libinput: ident) => {
        $libinput
            .dispatch()
            .unwrap()
    };
}



fn main() {
    let terminate = Arc::new(AtomicBool::default());

    main::register_termination_signals_handling(&terminate);

    // Connect to Sway to find active window centers
    // and animate cursor in javelin motion mode.
    let conn = swayipc::Connection::new()
        .expect("Cannot connect to Sway!");

    let mut libinput = Libinput::new_from_path(Interface);

    let args = cli::get_arguments();

    let pointer_device = libinput
        .path_add_device(&args.device_path) // Get device path first
        .expect("Cannot open pointer device");


    handle_events(conn, libinput, terminate, pointer_device, args)
        .unwrap()
}

mod main {
    use std::sync::{Arc, atomic::AtomicBool};

    pub fn register_termination_signals_handling(terminate: &Arc<AtomicBool>) {
        for sig in signal_hook::consts::TERM_SIGNALS {

            let sig_handler = Arc::clone(terminate);
            signal_hook::flag::register(*sig, sig_handler)
                .expect("Cannot register signal handler");
        }
    }
}



// Contains loop over libinput pointer events.
fn handle_events(
    mut conn: swayipc::Connection,
    mut libinput: Libinput,
    terminate: Arc<AtomicBool>,
    pointer_device: input::Device,
    args: cli::ArgContext,
) -> Result<(), Box<dyn Error>> {

    let cli::ArgContext {
        offsets,
        device_type,
        args: cli::Args {
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

    // Javelin sets some Sway settings for better experience
    conn.run_command(format!("
        focus_follows_mouse always
        mouse_warping container
        seat * hide_cursor {reload_msec}
        input type:{device_type} pointer_accel {javelin_acceleration}
    "))?;

    // Used to calculate mode timeouts
    let mut past_event_time = 0;
    let mut javelin = true;

    // Used to calculate animation timeouts
    let mut past_tremble_time = 0;
    let mut trembling = true;
    let mut trembles = handle_events::trembles();

    // Slightly moves cursor to create trembling animation effect
    let mut tremble = || trembles
        .next()
        .unwrap();


    loop {
        dispatch!(libinput);

        // Loops only over pointer events
        let Some(input::Event::Pointer(event)) = libinput.next() else {

            // This code used to finish trembling animation
            // by returning cursor to its expected position.
            if trembling {

                // There's some time between animation frames.
                // Dispatches libinput events every 6 milliseconds
                // instead of just sleeping during that time
                let mut wait_times = tremble_msec / (tremble_msec / 6);
                while wait_times > 0 {

                    dispatch!(libinput);

                    spin_sleep::sleep(Duration::from_millis(6));

                    wait_times -= 1;
                }

                let (x, y) = tremble();
                if (x, y) == handle_events::STOP_TREMBLING {
                    trembling = false;
                } else {
                    conn.run_command(format!("seat seat0 cursor move {x} {y}"))?;
                }
            } else {

                // Sleeps for 16 milliseconds before reading next
                // event in order to reduce resources usage.
                spin_sleep::sleep(Duration::from_millis(16));
            }

            // Also checks whether termination signal was sent.
            // If sent resets some Sway settings before exit.
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

                // When other device moves pointer this
                // switches javelin into slow mode.
                if event.device() != pointer_device {
                    past_event_time = past_event_time.saturating_sub(pointer_cooldown);
                    javelin = false;

                    continue
                }

                // If cursor didn't moved for `reload_msec` this sets
                // it's position at the center of active window so
                // the current swipe should start in fast mode from there.
                if delta_time > reload_msec {
                    conn.run_command(format!("
                        input type:{device_type} pointer_accel {javelin_acceleration}
                    "))?;

                    dispatch!(libinput);

                    let focused_window = conn
                        .get_tree()?
                        .find_focused(|n| n.nodes.is_empty())
                        .expect("Cannot get focused container");


                    let swayipc::Rect { mut x, mut y, width, height, .. } = focused_window.rect;
                    (x, y) = (
                        x + (width as f32 * x_split_reload) as i32,
                        y + (height as f32 * y_split_reload) as i32,
                    );

                    let swayipc::Node { app_id, window_properties, .. } = focused_window;

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

                // If pointer in fast mode didn't moved for
                // some time then switch it into slow mode.
                if javelin && delta_time > javelin_cooldown {
                    conn.run_command(format!("
                        input type:{device_type} pointer_accel {pointer_acceleration}
                    "))?;

                    javelin = false;

                    continue
                }

                // If pointer in slow mode didn't moved for
                // some time then switch it back to fast mode.
                if delta_time > pointer_cooldown {
                    conn.run_command(format!("
                        input type:{device_type} pointer_accel {javelin_acceleration}
                    "))?;

                    javelin = true;
                    trembling = true;

                    continue
                }

                // If pointer moves in fast mode and any timeout
                // didn't reached deadline this animates cursor.
                if javelin {
                    let delta_time = current_event_time.saturating_sub(past_tremble_time);
                    trembling = true;

                    if delta_time < tremble_msec {
                        // Not enough time between animation frames
                        continue
                    }

                    past_tremble_time = current_event_time;

                    let (mut x, mut y) = tremble();

                    // Skip through STOP_TREMBLING (0, 0) coordinates.
                    if (x, y) == handle_events::STOP_TREMBLING {
                        (x, y) = tremble();
                    }

                    conn.run_command(format!("seat seat0 cursor move {x} {y}"))?;
                }
            },

            // Enter into slow mode on scroll events.
            __::ScrollContinuous(_) | __::ScrollFinger(_) | __::ScrollWheel(_) => {

                past_event_time = event.time();

                javelin = false;
            },

            _ => {}
        }
    }
}


mod handle_events {
    pub const STOP_TREMBLING: (i32, i32) = (0, 0);

    // This function generates (x, y) distances
    // to animate cursor in fast mode.
    //
    // Cursor movement resembles the following pattern:
    //
    //  third  first
    //  2 < 1  1 > 2
    //  v   ^  ^   v
    //  3 > 0  0 < 3
    //  second fourth
    //  1 < 0  0 > 1
    //  v   ^  ^   v
    //  2 > 3  3 < 2
    //
    pub fn trembles() -> impl Iterator<Item = (i32, i32)> {
        let direction = (0..4)
            .cycle();

        // We start with array of numbers with some entropy.
        [ 15, 8, 10, 9, 16, 7, 18, 13, 6, 11, 19, 12, 17 ]
            // Then for each number
            .into_iter()
            .scan(direction, |dir, dist| {

                // Generate a "square" movement
                // with some "direction" on x:y coordinates.
                // Each movement ends with (0, 0) sequence
                // which means end of movement in loop.
                let movements = match dir.next().unwrap() {
                    0 => [(0, dist), (dist, 0), (0, -dist), (-dist, 0), STOP_TREMBLING] ,
                    1 => [(-dist, 0), (0, -dist), (dist, 0), (0, dist), STOP_TREMBLING] ,
                    2 => [(0, dist), (-dist, 0), (0, -dist), (dist, 0), STOP_TREMBLING] ,
                    3 => [(dist, 0), (0, -dist), (-dist, 0), (0, dist), STOP_TREMBLING] ,
                    _ => unreachable!()
                };

                Some(movements)
            })
            .flat_map(|x| x)
            .cycle()
    }
}


mod cli {
    use clap::Parser;
    use std::collections::HashMap;

    pub fn get_arguments() -> ArgContext {
        let args = Args::parse();
        println!("{args:#?}");

        // Some windows have sidebars, for example Visual Studio Code
        // but users are usually focused on main window. This visually
        // shifts center of window, so that's why javelin allows to
        // provide offsets from center.
        let offsets = args.offsets.iter()
            .map(get_arguments::parse_offset_value)
            .collect();

        let device_path = args.device
            .clone()
            .unwrap_or_else(get_arguments::detect_touchpad_device);

        // When user provides --device=/dev/input/device it's impossible
        // to detect type of device, for example "touchpad" or "pointer"
        // required in Sway IPC. So it must be either provided by user.
        let device_type = args.device_type
            .clone()
            .unwrap_or("touchpad".to_string());

        ArgContext {
            args,
            offsets,
            device_path,
            device_type,
        }
    }

    mod get_arguments {
        // Parses offset value in app:x:y format
        pub fn parse_offset_value(arg: &String) -> (String, (i32, i32)) {
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

        // Parses `libinput list-devices` output to find touchpad's /dev/input/* path
        pub fn detect_touchpad_device() -> String {
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
    }


    pub struct ArgContext {
        pub args: Args,
        pub device_type: String,
        pub device_path: String,
        pub offsets: HashMap<String, (i32, i32)>
    }

    #[derive(Parser, Debug)]
    #[clap(author, version)]
    pub struct Args {

        /// Path to /dev/input/ device to use. {n}
        /// With this argument device_type must be also provided
        /// otherwise --device-type=touchpad is implied
        #[clap(long)]
        pub device: Option<String>,

        /// Type of --device. Read `man sway-input` for available types
        #[clap(long)]
        pub device_type: Option<String>,

        /// Pointer acceleration in slow mode for Sway
        #[clap(long, default_value = "-0.2")]
        pub pointer_acceleration: f32,

        /// Pointer acceleration in fast mode for Sway
        #[clap(long, default_value = "0.8")]
        pub javelin_acceleration: f32,

        /// Time of pointer rest before swithcing
        /// from slow mode into fast mode
        #[clap(long, default_value = "400")]
        pub pointer_cooldown: u32,

        /// Time of pointer rest before swithcing
        /// from fast mode into slow mode
        #[clap(long, default_value = "32")]
        pub javelin_cooldown: u32,

        /// Time between cursor animation "frames"
        #[clap(long, default_value = "32")]
        pub tremble_msec: u32,

        /// Time after which pointer will be hidden
        /// and next movement will start from the
        /// center of currently active window
        #[clap(long, default_value = "4096")]
        pub reload_msec: u32,

        /// Used to find x position of center of window. {n}
        /// Set to 0 to reload pointer from left side
        /// and 1 to reload from right side
        #[clap(long, default_value = "0.5")]
        pub x_split_reload: f32,

        /// Used to find y position of center of window. {n}
        /// Set to 0 to reload pointer from top side
        /// and 1 to reload from bottom side
        #[clap(long, default_value = "0.5")]
        pub y_split_reload: f32,

        /// Some applications have sidebars which shifts
        /// their center at the left/right side. {n}
        /// If you want to reload from the center of content
        /// then specify which window and how far center is
        /// shifted using this argument. {n}
        /// Format is app:x:y where {n}
        ///  - app can be either app_id or class or title
        ///    refer to `man 5 sway: CRITERIA` for information {n}
        ///  - x and y can be negative numbers
        pub offsets: Vec<String>
    }
}

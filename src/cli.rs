
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
    #[clap(long, default_value = "1.0")]
    pub javelin_acceleration: f32,

    /// Time of pointer rest before swithcing
    /// from slow mode into fast mode
    #[clap(long, default_value = "400")]
    pub pointer_cooldown: u32,

    /// Time of pointer rest before swithcing
    /// from fast mode into slow mode
    #[clap(long, default_value = "48")]
    pub javelin_cooldown: u32,

    /// Don't hide cursor on javelin reload
    #[clap(long)]
    pub do_not_hide_cursor: bool,

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

mod cli;
mod core;
mod event_controller;
mod input_controller;
mod logging;
#[cfg(feature = "tracing")]
mod trace;

use std::{
    cell::RefCell,
    fs::File,
    io::{prelude::*, stdin},
    process::exit,
    rc::Rc,
    thread,
};

use event_controller::{style::TermionStyles, window::TermionLayout, EventController, Styles};
use failure::{format_err, Error};
use input_controller::{keyboard::TermionKeyboard, Config, InputController};
use serde_json::json;
use xi_rpc::{Peer, RpcLoop};

fn setup_logger() {
    let logging_path = dirs::home_dir()
        .expect("failed to retrieve the home dir")
        .join(".local/share/vixy/vixi.log");

    logging::setup(&logging_path).expect("failed to set the logger")
}

fn setup_config(core: &dyn Peer) -> Result<Config, Error> {
    let config_dir = dirs::config_dir().ok_or_else(|| format_err!("config dir not found"))?;

    let mut xi_config_dir = config_dir.clone();
    xi_config_dir.push("xi");
    core.send_rpc_notification(
        "client_started",
        &json!({ "config_dir": xi_config_dir.to_str().unwrap(), }),
    );

    let vixi_config_dir = config_dir.join("vixi");
    let vixi_keyboard_config_file = vixi_config_dir.join("keyboard.toml");

    let config = if vixi_keyboard_config_file.is_file() {
        let mut keyboard_config_file = File::open(vixi_keyboard_config_file)?;
        let mut keyboard_config_contents = String::new();
        keyboard_config_file.read_to_string(&mut keyboard_config_contents)?;
        toml::from_str(&keyboard_config_contents)?
    } else {
        Config::default()
    };

    Ok(config)
}

fn main() {
    let matches = cli::build().get_matches();

    let file_path = matches
        .value_of("file")
        .expect("failed to retrieve cli value");

    setup_logger();

    #[cfg(feature = "tracing")]
    trace::start_tracer();

    let (client_to_core_writer, core_to_client_reader, client_to_client_writer) =
        core::start_xi_core();
    let mut front_event_loop = RpcLoop::new(client_to_core_writer);

    let raw_peer = front_event_loop.get_raw_peer();
    let config = match setup_config(&raw_peer) {
        Ok(config) => config,
        Err(err) => {
            println!("failed to load the configuration: {}", err);
            exit(1);
        }
    };

    let child = thread::spawn(move || {
        let layout = TermionLayout::new();

        let styles: Rc<RefCell<Box<dyn Styles>>> =
            Rc::new(RefCell::new(Box::new(TermionStyles::new())));

        let mut event_handler = EventController::new(Box::new(layout), styles.clone());
        front_event_loop
            .mainloop(|| core_to_client_reader, &mut event_handler)
            .unwrap();
    });

    let mut input_controller = InputController::new(
        Box::new(TermionKeyboard::from_reader(stdin())),
        client_to_client_writer,
        &config,
    );

    if let Err(err) = input_controller.open_file(&raw_peer, file_path) {
        println!("failed to open {}: {}", file_path, err);
        exit(1);
    }

    if let Err(err) = input_controller.start_keyboard_event_loop(&raw_peer) {
        println!("an error occured: {}", err);
        exit(1);
    }

    child.join().unwrap();

    #[cfg(feature = "tracing")]
    trace::write_trace_dump_into("./trace.out")
}

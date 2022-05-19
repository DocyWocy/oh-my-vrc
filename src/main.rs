use tokio::task;
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;
use structopt::StructOpt;
use url::Url;
use nannou_osc as osc;
use nannou_osc::rosc::OscType;
use buttplug::{
    client::{ButtplugClient, ButtplugClientDevice, ButtplugClientEvent,
             device::VibrateCommand},
    connector::{ButtplugRemoteClientConnector, ButtplugWebsocketClientTransport},
    core::messages::serializer::ButtplugClientJSONSerializer,
    //core::messages::SingleMotorVibrateCmd,
    //core::messages::ButtplugDeviceCommandMessageUnion::SingleMotorVibrateCmd,
    //core::messages::ButtplugDeviceMessageType::SingleMotorVibrateCmd,
};
use anyhow::{bail, Result, Error};
use tracing::{debug, info, warn, error};
use std::{thread, time};
use std::collections::HashMap;


const DEVICES_ALL: &str = "all";
const DEVICES_LAST: &str = "last";

#[derive(StructOpt)]
/// Control https://buttplug.io/ devices via OSC
struct CliArgs {
    #[structopt(long, default_value = "ws://127.0.0.1:12345")]
    intiface_connect: Url,

    #[structopt(long, default_value = "udp://0.0.0.0:9000")]
    osc_listen: Url,

    #[structopt(long = "log-level", env = "RUST_LOG", default_value = "debug")]
    rust_log: String,
}

#[tokio::main]
async fn main() -> Result<()> {

    let args = CliArgs::from_args();
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new(args.rust_log))
        .with_thread_names(true)
        .init();
    info!("main!");
    let osc_listen_host_port = validate_osc_listen_url(&args.osc_listen);
    let (devices_r, devices_w) = evmap::new();
    task::spawn_blocking(move || {
        info!("Starting OSC Server ({})", osc_listen_host_port);
        osc_listen(&osc_listen_host_port, devices_r);
    });
    let devices_m = Arc::new(Mutex::new(devices_w));
    info!("main! devices_m");
    loop {
        info!("main! loop!");
        let address = String::from(args.intiface_connect.as_str());
        let devices = devices_m.clone();
        let _ = task::spawn(intiface_connect(address, devices)).await;
    }
}

async fn intiface_connect(address: String, devices: Arc<Mutex<evmap::WriteHandle<&str, Device>>>) -> Result<()> {
    info!("Starting Intiface Client ({})", address);
    // https://buttplug-developer-guide.docs.buttplug.io/writing-buttplug-applications/device-enum.html#device-connection-events-and-storage
    // > The server could already be running and have devices connected to it. In this case, the Client will emit DeviceAdded events on successful connection.
    // > This means you will want to have your event handlers set up BEFORE connecting, in order to catch these messages.

    let client = ButtplugClient::new("buttplug-osc");
    let mut event_stream = client.event_stream();
    let event_loop = async {
        while let Some(event) = event_stream.next().await {
            match event {
                ButtplugClientEvent::DeviceAdded(device) => {
                    let mut devices = devices.lock().expect("unexpected");
                    let name = Box::leak(
                        normalize_device_name(&device.name).into_boxed_str());
                    devices.update(name, Device { device: device.clone() });
                    devices.update(DEVICES_LAST, Device { device: device.clone() });
                    devices.refresh();
                    info!("[{}] added", name);
                }
                ButtplugClientEvent::DeviceRemoved(device) => {
                    warn!("[{}] removed", normalize_device_name(&device.name));
                    // rescanning, maybe a temporary disconnect
                    let _ = client.stop_scanning().await;
                    let _ = client.start_scanning().await;
                }
                ButtplugClientEvent::ServerDisconnect => {
                    bail!("ServerDisconnect");
                }
                _ => {}
            }
        };
        Ok::<(), Error>(())
    };

    let connector = ButtplugRemoteClientConnector::<
        ButtplugWebsocketClientTransport,
        ButtplugClientJSONSerializer,
    >::new(ButtplugWebsocketClientTransport::new_insecure_connector(&address));

    client.connect(connector).await?;
    client.start_scanning().await?;
    event_loop.await
}

fn normalize_device_name(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric()).collect::<String>()
}

fn osc_listen(host_port: &str, devices: evmap::ReadHandle<&'static str, Device>) {
    let rx = osc::Receiver::bind_to(host_port).expect("Invalid --osc-listen: couldn't bind socket");
    for packet in rx.iter() {
        let messages = packet.0.into_msgs();
        for message in messages {
            if let Some(broadcast) = validate_osc_message(message) {
                if let Some(iter) = filter_devices(&broadcast.devices_set[..], &devices) {
                    for device in iter {
                        let device_name = normalize_device_name(&device.name);
                        let device = device.clone();
                        match broadcast.command {
                            Command::Vibrate(speed) => {
                                task::spawn(async move {
                                    debug!("[{}] adjusting vibration", device_name);
                                    device.vibrate(VibrateCommand::Speed(speed)).await.map_err(|e|
                                        error!("{:?}", e)
                                    )
                                })
                            }
                            Command::Stop => {
                                task::spawn(async move {
                                    debug!("[{}] stopping", device_name);
                                    device.stop().await.map_err(|e|
                                        error!("{:?}", e)
                                    )
                                })
                            }
                            Command::Test(index) => {
                                task::spawn(async move {
                                    debug!("[{}] testing", device_name);
                                    println!("tested");
                                    let mut feature :u32 = index as u32;
                                    let millis = time::Duration::from_millis(33); //30.3hz update rate 30 times a second

                                    thread::sleep(millis);

                                    for x in 1..150 { // about 10 seconds? ramp up then stop
                                        speed_cmd = 1.0/(x as f64); //SingleMotorVibrateCmd
                                        //SingleMotorVibrateCmd::new(1,speed_cmd);
                                        //SingleMotorVibrateCmd();DeviceIndex
                                        //let s: HashMap<u32, f64> = {index, speed_cmd};

                                        device.vibrate(VibrateCommand::SpeedMap(HashMap::from([(feature, speed_cmd)]))).await.map_err(|e| error!("{:?}", e)).expect("unable to perform VibrateCommand::SpeedMap");
                                        //device.vibrate(VibrateCommand::Speed(speed_cmd)).await.map_err(|e| error!("{:?}", e));
                                        thread::sleep(millis);
                                    }

                                    thread::sleep(millis);
                                    device.vibrate(VibrateCommand::SpeedMap(HashMap::from([(feature, 0.0)]))).await.map_err(|e| error!("{:?}", e))
                                })
                            }
                        };
                    }
                }
            }
        }
    }
}

fn filter_devices<'d>(set: &str, devices: &'d evmap::ReadHandle<&str, Device>) -> Option<impl Iterator<Item=evmap::ReadGuard<'d, Device>>> {
    let mut result = Vec::new();

    if let Some(device) = devices.get_one(set) {
        result.push(device);
    } else {
        for (k, _) in devices.read()?.iter() {
            if (set == DEVICES_ALL || k.starts_with(set)) && (*k != DEVICES_LAST) {
                result.push(devices.get_one(k).expect("unexpected"));
            }
        }
    }

    Some(result.into_iter())
}

fn validate_osc_message(message: osc::Message) -> Option<CommandBroadcast> {
    let path = message.addr.split('/').collect::<Vec<&str>>();
    let invalid = |error: &str| {
        warn!("[{}] {}", message.addr, error);
        None::<CommandBroadcast>
    };
    let pass_error = |error: &str| {
        //warn!("",);
        None::<CommandBroadcast>
    };
    /*
    string = message.addr.split('/');
    for s in string {
        //info!("{}", s);
    }*/

    match path.get(3) { //device
        Some(&"devices") => {
            match path.get(5) { //skips name and gets <command>
                Some(&"stop") => {

                    debug!("[{}]", message.addr);
                    Some(CommandBroadcast {
                        devices_set: String::from(path[4]),
                        command: Command::Stop,
                    })
                }
                Some(&"test") => {
                    match path.get(6) { //gets <argument>
                        Some(&"index") => {
                            match message.args {
                                Some(ref message_args) => {
                                    let index: i32 = match message_args.get(0) {
                                        Some(OscType::Int(x)) => {
                                            *x
                                        }
                                        Some(OscType::Int(x)) => {
                                            (*x).into()
                                        }
                                        _ => {
                                            return invalid(&format!("invalid argument value: {:?}", message_args[0]));
                                        }
                                    };
                                    debug!("[{}] {}", message.addr, index);
                                    debug!("[{}]", message.addr);
                                    Some(CommandBroadcast {
                                        devices_set: String::from(path[4]),
                                        command: Command::Test(index),
                                    })
                                }
                                None => invalid("invalid argument value: none")
                            }
                        }
                        _ => invalid("invalid argument name")
                    }

                }
                Some(&"vibrate") => {
                    match path.get(6) { //gets <argument>
                        Some(&"speed") => {
                            match message.args {
                                Some(ref message_args) => {
                                    let speed: f64 = match message_args.get(0) {
                                        Some(OscType::Double(x)) => {
                                            *x
                                        }
                                        Some(OscType::Float(x)) => {
                                            (*x).into()
                                        }
                                        _ => {
                                            return invalid(&format!("invalid argument value: {:?}", message_args[0]));
                                        }
                                    };
                                    debug!("[{}] {}", message.addr, speed);
                                    Some(CommandBroadcast {
                                        devices_set: String::from(path[4]),
                                        command: Command::Vibrate(speed),
                                    })
                                }
                                None => invalid("invalid argument value: none")
                            }
                        }
                        _ => invalid("invalid argument name")
                    }
                }
                _ => invalid("invalid command")
            }
        }
        _ => pass_error("invalid message")
    }
}

fn validate_osc_listen_url(osc_listen_url: &Url) -> String {
    match osc_listen_url.scheme() {
        "udp" => {}
        _ => {
            unimplemented!("Invalid --osc-listen: only OSC-over-UDP is supported currently");
        }
    }
    let osc_listen_host = osc_listen_url.host().expect("Invalid --osc-listen");
    let osc_listen_port = osc_listen_url.port().expect("Invalid --osc-listen");
    format!("{}:{}", osc_listen_host, osc_listen_port)
}

type Speed = f64;
type Index = i32;
enum Command {
    Stop,
    Vibrate(Speed),
    Test(Index),
}

struct CommandBroadcast {
    devices_set: String,
    command: Command,
}


// evmap required Hash trait which was not implemented by ButtplugClientDevice

#[derive(Debug, Eq, Clone, evmap_derive::ShallowCopy)]
struct Device {
    device: Arc<ButtplugClientDevice>
}

impl std::hash::Hash for Device {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.device.name.hash(state);
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.device.eq(&other.device)
    }
}

impl std::ops::Deref for Device {
    type Target = Arc<ButtplugClientDevice>;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}

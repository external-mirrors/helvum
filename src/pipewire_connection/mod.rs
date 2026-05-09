// Copyright 2021 Tom A. Wagner <tom.a.wagner@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License version 3 as published by
// the Free Software Foundation.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: GPL-3.0-only

mod state;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use libspa::{
    param::{ParamInfoFlags, ParamType},
    utils::dict::DictRef,
};
use log::{debug, error, info, warn};
use pipewire::{
    context::ContextRc,
    core::CoreRc,
    keys,
    link::{Link, LinkChangeMask, LinkListener, LinkState},
    main_loop::MainLoopRc,
    node::{Node, NodeListener},
    port::{Port, PortChangeMask, PortListener},
    registry::{GlobalObject, RegistryRc},
    types::ObjectType,
};

use crate::{GtkMessage, LinkId, MediaType, NodeId, NodeType, PipewireMessage, PortId};
use state::{Item, State};

enum ProxyItem {
    Node {
        _proxy: Node,
        _listener: NodeListener,
    },
    Port {
        proxy: Port,
        _listener: PortListener,
    },
    Link {
        _proxy: Link,
        _listener: LinkListener,
    },
}

/// The "main" function of the pipewire thread.
pub(super) fn thread_main(
    gtk_sender: async_channel::Sender<PipewireMessage>,
    mut pw_receiver: pipewire::channel::Receiver<GtkMessage>,
) {
    let mainloop = MainLoopRc::new(None).expect("Failed to create mainloop");
    let context = ContextRc::new(&mainloop, None).expect("Failed to create context");
    let is_stopped = Rc::new(Cell::new(false));
    let mut is_connecting = false;

    while !is_stopped.get() {
        // Try to connect
        let core = match context.connect_rc(None) {
            Ok(core) => core,
            Err(_) => {
                if !is_connecting {
                    is_connecting = true;
                    gtk_sender
                        .send_blocking(PipewireMessage::Connecting)
                        .expect("Failed to send message");
                }

                // If connection failed, try again in 200ms
                let interval = Some(Duration::from_millis(200));
                let ml = mainloop.clone();
                let timer = mainloop.loop_().add_timer(move |_| {
                    ml.quit();
                });
                timer.update_timer(interval, None).into_result().unwrap();

                let ml2 = mainloop.clone();
                let is_stopped2 = is_stopped.clone();
                let receiver = pw_receiver.attach(mainloop.loop_(), move |msg| {
                    if let GtkMessage::Terminate = msg {
                        is_stopped2.set(true);
                        ml2.quit();
                    }
                });

                mainloop.run();
                pw_receiver = receiver.deattach();
                continue;
            }
        };

        if is_connecting {
            is_connecting = false;
            gtk_sender
                .send_blocking(PipewireMessage::Connected)
                .expect("Failed to send message");
        }

        let registry = core.get_registry_rc().expect("Failed to get registry");

        let proxies: Rc<RefCell<HashMap<u32, ProxyItem>>> = Rc::new(RefCell::new(HashMap::new()));
        let state = Rc::new(RefCell::new(State::new()));

        // Attach receiver to process GTK→PW messages
        let ml3 = mainloop.clone();
        let core2 = core.clone();
        let registry2 = registry.clone();
        let state2 = state.clone();
        let is_stopped2 = is_stopped.clone();
        let receiver = pw_receiver.attach(mainloop.loop_(), move |msg| match msg {
            GtkMessage::ToggleLink { port_from, port_to } => {
                toggle_link(port_from, port_to, &core2, &registry2, &state2)
            }
            GtkMessage::EnsureLink { port_from, port_to } => {
                ensure_link(port_from, port_to, &core2, &registry2, &state2)
            }
            GtkMessage::Terminate => {
                is_stopped2.set(true);
                ml3.quit();
            }
        });

        // Listen for core errors (e.g. disconnection)
        let gtk_sender2 = gtk_sender.clone();
        let ml4 = mainloop.clone();
        let _listener = core
            .add_listener_local()
            .error(move |id, _seq, res, message| {
                if id != 0 {
                    return;
                }
                if res == -libc::EPIPE {
                    gtk_sender2
                        .send_blocking(PipewireMessage::Disconnected)
                        .expect("Failed to send message");
                    ml4.quit();
                } else {
                    use libspa::utils::result::SpaResult;
                    let serr = SpaResult::from_c(res).into_result().unwrap_err();
                    error!("Pipewire Core received error {serr}: {message}");
                }
            })
            .register();

        // Listen for registry events (nodes, ports, links appearing / disappearing)
        let gtk_sender3 = gtk_sender.clone();
        let registry3 = registry.clone();
        let proxies2 = proxies.clone();
        let state3 = state.clone();
        let proxies_remove = proxies.clone();
        let state_remove = state.clone();
        let gtk_sender4 = gtk_sender.clone();
        let _listener = registry
            .add_listener_local()
            .global(move |global| match global.type_ {
                ObjectType::Node => {
                    handle_node(global, &gtk_sender3, &registry3, &proxies2, &state3)
                }
                ObjectType::Port => {
                    handle_port(global, &gtk_sender3, &registry3, &proxies2, &state3)
                }
                ObjectType::Link => {
                    handle_link(global, &gtk_sender3, &registry3, &proxies2, &state3)
                }
                _ => {}
            })
            .global_remove(move |id| {
                if let Some(item) = state_remove.borrow_mut().remove(id) {
                    gtk_sender4
                        .send_blocking(match item {
                            Item::Node => PipewireMessage::NodeRemoved { id: NodeId(id) },
                            Item::Port { node_id } => PipewireMessage::PortRemoved {
                                id: PortId(id),
                                node_id,
                            },
                            Item::Link { .. } => PipewireMessage::LinkRemoved { id: LinkId(id) },
                        })
                        .expect("Failed to send message");
                } else {
                    warn!(
                        "Attempted to remove item with id {} that is not saved in state",
                        id
                    );
                }
                proxies_remove.borrow_mut().remove(&id);
            })
            .register();

        mainloop.run();
        pw_receiver = receiver.deattach();
    }
}

/// Get the nicest possible name for the node, using a fallback chain of possible name attributes
fn get_node_name(props: &DictRef) -> &str {
    props
        .get(&keys::NODE_DESCRIPTION)
        .or_else(|| props.get(&keys::NODE_NICK))
        .or_else(|| props.get(&keys::NODE_NAME))
        .unwrap_or_default()
}

/// Handle a new node being added
fn handle_node(
    node: &GlobalObject<&DictRef>,
    sender: &async_channel::Sender<PipewireMessage>,
    registry: &RegistryRc,
    proxies: &Rc<RefCell<HashMap<u32, ProxyItem>>>,
    state: &Rc<RefCell<State>>,
) {
    let props = node
        .props
        .as_ref()
        .expect("Node object is missing properties");

    let name = get_node_name(props).to_string();
    let internal_name = props
        .get(&pipewire::keys::NODE_NAME)
        .unwrap_or(&name)
        .to_string();
    let media_class = |class: &str| {
        if class.contains("Sink") || class.contains("Input") {
            Some(NodeType::Input)
        } else if class.contains("Source") || class.contains("Output") {
            Some(NodeType::Output)
        } else {
            None
        }
    };

    let node_type = props
        .get("media.category")
        .and_then(|class| {
            if class.contains("Duplex") {
                None
            } else {
                props.get("media.class").and_then(media_class)
            }
        })
        .or_else(|| props.get("media.class").and_then(media_class));

    state.borrow_mut().insert(node.id, Item::Node);

    sender
        .send_blocking(PipewireMessage::NodeAdded {
            id: NodeId(node.id),
            name,
            internal_name,
            node_type,
        })
        .expect("Failed to send message");

    let proxy: Node = registry.bind(node).expect("Failed to bind to node proxy");
    let sender2 = sender.clone();
    let proxies2 = proxies.clone();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            handle_node_info(info, &sender2, &proxies2);
        })
        .register();

    proxies.borrow_mut().insert(
        node.id,
        ProxyItem::Node {
            _proxy: proxy,
            _listener: listener,
        },
    );
}

fn handle_node_info(
    info: &pipewire::node::NodeInfoRef,
    sender: &async_channel::Sender<PipewireMessage>,
    proxies: &Rc<RefCell<HashMap<u32, ProxyItem>>>,
) {
    debug!("Received node info: {:?}", info);

    let id = info.id();
    let proxies = proxies.borrow();
    let Some(ProxyItem::Node { .. }) = proxies.get(&id) else {
        error!("Received info on unknown node with id {id}");
        return;
    };

    let props = info.props().expect("NodeInfo object is missing properties");
    if let Some(media_name) = props.get(&keys::MEDIA_NAME) {
        let name = get_node_name(props).to_string();

        sender
            .send_blocking(PipewireMessage::NodeNameChanged {
                id: NodeId(id),
                name,
                media_name: media_name.to_string(),
            })
            .expect("Failed to send message");
    }
}

/// Handle a new port being added
fn handle_port(
    port: &GlobalObject<&DictRef>,
    sender: &async_channel::Sender<PipewireMessage>,
    registry: &RegistryRc,
    proxies: &Rc<RefCell<HashMap<u32, ProxyItem>>>,
    state: &Rc<RefCell<State>>,
) {
    let port_id = port.id;
    let proxy: Port = registry.bind(port).expect("Failed to bind to port proxy");

    let sender2 = sender.clone();
    let proxies2 = proxies.clone();
    let state2 = state.clone();
    let sender3 = sender.clone();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            handle_port_info(info, &proxies2, &state2, &sender2);
        })
        .param(move |_, param_id, _, _, param| {
            if param_id == ParamType::EnumFormat {
                handle_port_enum_format(port_id, param, &sender3)
            }
        })
        .register();

    proxies.borrow_mut().insert(
        port.id,
        ProxyItem::Port {
            proxy,
            _listener: listener,
        },
    );
}

fn handle_port_info(
    info: &pipewire::port::PortInfoRef,
    proxies: &Rc<RefCell<HashMap<u32, ProxyItem>>>,
    state: &Rc<RefCell<State>>,
    sender: &async_channel::Sender<PipewireMessage>,
) {
    debug!("Received port info: {:?}", info);

    let id = info.id();
    let proxies = proxies.borrow();
    let Some(ProxyItem::Port { proxy, .. }) = proxies.get(&id) else {
        log::error!("Received info on unknown port with id {id}");
        return;
    };

    let mut state = state.borrow_mut();

    if let Some(Item::Port { .. }) = state.get(id) {
        // Info was an update, figure out if we should notify the GTK thread
        if info.change_mask().contains(PortChangeMask::PARAMS) {
            // TODO: React to param changes
        }
    } else {
        // First time we get info. We can now notify the gtk thread of a new port.
        let props = info.props().expect("Port object is missing properties");
        let name = props.get("port.name").unwrap_or_default().to_string();
        let node_id = NodeId(
            props
                .get("node.id")
                .expect("Port has no node.id property!")
                .parse()
                .expect("Could not parse node.id property"),
        );

        state.insert(id, Item::Port { node_id });

        let params = info.params();
        let enum_format_info = params
            .iter()
            .find(|param| param.id() == ParamType::EnumFormat);
        if let Some(enum_format_info) = enum_format_info {
            if enum_format_info.flags().contains(ParamInfoFlags::READ) {
                proxy.enum_params(0, Some(ParamType::EnumFormat), 0, u32::MAX);
            }
        }

        sender
            .send_blocking(PipewireMessage::PortAdded {
                id: PortId(id),
                node_id,
                name,
                direction: info.direction(),
            })
            .expect("Failed to send message");
    }
}

fn handle_port_enum_format(
    port_id: u32,
    param: Option<&libspa::pod::Pod>,
    sender: &async_channel::Sender<PipewireMessage>,
) {
    let media_type = param
        .and_then(|param| libspa::param::format_utils::parse_format(param).ok())
        .map(|(media_type, _media_subtype)| media_type)
        .unwrap_or(MediaType::Unknown);

    sender
        .send_blocking(PipewireMessage::PortFormatChanged {
            id: PortId(port_id),
            media_type,
        })
        .expect("Failed to send message")
}

/// Handle a new link being added
fn handle_link(
    link: &GlobalObject<&DictRef>,
    sender: &async_channel::Sender<PipewireMessage>,
    registry: &RegistryRc,
    proxies: &Rc<RefCell<HashMap<u32, ProxyItem>>>,
    _state: &Rc<RefCell<State>>,
) {
    debug!(
        "New link (id:{}) appeared, setting up info listener.",
        link.id
    );

    let proxy: Link = registry.bind(link).expect("Failed to bind to link proxy");
    let sender2 = sender.clone();
    let state_link = _state.clone();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            handle_link_info(info, &state_link, &sender2);
        })
        .register();

    proxies.borrow_mut().insert(
        link.id,
        ProxyItem::Link {
            _proxy: proxy,
            _listener: listener,
        },
    );
}

fn handle_link_info(
    info: &pipewire::link::LinkInfoRef,
    state: &Rc<RefCell<State>>,
    sender: &async_channel::Sender<PipewireMessage>,
) {
    debug!("Received link info: {:?}", info);

    let id = info.id();

    let mut state = state.borrow_mut();
    if let Some(Item::Link { .. }) = state.get(id) {
        if info.change_mask().contains(LinkChangeMask::STATE) {
            sender
                .send_blocking(PipewireMessage::LinkStateChanged {
                    id: LinkId(id),
                    active: matches!(info.state(), LinkState::Active),
                })
                .expect("Failed to send message");
        }
        if info.change_mask().contains(LinkChangeMask::FORMAT) {
            sender
                .send_blocking(PipewireMessage::LinkFormatChanged {
                    id: LinkId(id),
                    media_type: get_link_media_type(info),
                })
                .expect("Failed to send message");
        }
    } else {
        let port_from = PortId(info.output_port_id());
        let port_to = PortId(info.input_port_id());

        state.insert(id, Item::Link { port_from, port_to });

        sender
            .send_blocking(PipewireMessage::LinkAdded {
                id: LinkId(id),
                port_from,
                port_to,
                active: matches!(info.state(), LinkState::Active),
                media_type: get_link_media_type(info),
            })
            .expect("Failed to send message");
    }
}

/// Toggle a link between the two specified ports.
fn toggle_link(
    port_from: PortId,
    port_to: PortId,
    core: &CoreRc,
    registry: &RegistryRc,
    state: &Rc<RefCell<State>>,
) {
    let state = state.borrow();
    if let Some(id) = state.get_link_id(port_from, port_to) {
        info!("Requesting removal of link with id {}", id.0);
        registry.destroy_global(id.0);
    } else {
        info!(
            "Requesting creation of link from port id:{} to port id:{}",
            port_from.0, port_to.0
        );

        let Some(node_from) = state.get_node_of_port(port_from) else {
            warn!("Requested port {} not in state", port_from);
            return;
        };
        let Some(node_to) = state.get_node_of_port(port_to) else {
            warn!("Requested port {} not in state", port_to);
            return;
        };

        if let Err(e) = core.create_object::<Link>(
            "link-factory",
            &pipewire::properties::properties! {
                "link.output.node" => node_from.to_string(),
                "link.output.port" => port_from.to_string(),
                "link.input.node" => node_to.to_string(),
                "link.input.port" => port_to.to_string(),
                "object.linger" => "1"
            },
        ) {
            warn!("Failed to create link: {}", e);
        }
    }
}

fn get_link_media_type(link_info: &pipewire::link::LinkInfoRef) -> MediaType {
    link_info
        .format()
        .and_then(|format| libspa::param::format_utils::parse_format(format).ok())
        .map(|(media_type, _media_subtype)| media_type)
        .unwrap_or(MediaType::Unknown)
}

/// Ensure a link exists between the two specified ports.
fn ensure_link(
    port_from: PortId,
    port_to: PortId,
    core: &CoreRc,
    _registry: &RegistryRc,
    state: &Rc<RefCell<State>>,
) {
    let state_borrow = state.borrow();
    if state_borrow.get_link_id(port_from, port_to).is_some() {
        log::info!("Link from port {} to {} already exists, skipping ensure_link", port_from, port_to);
        return;
    }

    log::info!(
        "Requesting creation of link from port id:{} to port id:{}",
        port_from.0, port_to.0
    );

    let Some(node_from) = state_borrow.get_node_of_port(port_from) else {
        log::warn!("Requested port {} not in state", port_from);
        return;
    };
    let Some(node_to) = state_borrow.get_node_of_port(port_to) else {
        log::warn!("Requested port {} not in state", port_to);
        return;
    };

    if let Err(e) = core.create_object::<Link>(
        "link-factory",
        &pipewire::properties::properties! {
            "link.output.node" => node_from.to_string(),
            "link.output.port" => port_from.to_string(),
            "link.input.node" => node_to.to_string(),
            "link.input.port" => port_to.to_string(),
            "object.linger" => "1"
        },
    ) {
        log::warn!("Failed to create link: {}", e);
    }
}


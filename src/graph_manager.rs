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

use adw::{glib, prelude::*, subclass::prelude::*};

use pipewire::channel::Sender as PwSender;

use crate::{ui::graph::GraphView, GtkMessage, PipewireMessage};

mod imp {
    use super::*;

    use std::{cell::OnceCell, cell::RefCell, collections::HashMap};

    use crate::{ui::graph, LinkId, MediaType, NodeId, NodeType, PortId};

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::GraphManager)]
    pub struct GraphManager {
        #[property(get, set, construct_only, nullable)]
        pub graph: RefCell<Option<crate::ui::graph::GraphView>>,

        #[property(get, set, construct_only, nullable)]
        pub connection_banner: RefCell<Option<adw::Banner>>,

        pub pw_sender: OnceCell<PwSender<crate::GtkMessage>>,
        pub items: RefCell<HashMap<u32, glib::Object>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GraphManager {
        const NAME: &'static str = "HelvumGraphManager";
        type Type = super::GraphManager;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for GraphManager {}

    impl GraphManager {
        pub fn attach_receiver(&self, receiver: async_channel::Receiver<crate::PipewireMessage>) {
            let obj = self.obj().clone();
            glib::MainContext::default().spawn_local(async move {
                while let Ok(msg) = receiver.recv().await {
                    let imp = obj.imp();
                    match msg {
                        PipewireMessage::NodeAdded {
                            id,
                            name,
                            node_type,
                        } => imp.add_node(id, name.as_str(), node_type),
                        PipewireMessage::NodeNameChanged {
                            id,
                            name,
                            media_name,
                        } => imp.node_name_changed(id, &name, &media_name),
                        PipewireMessage::PortAdded {
                            id,
                            node_id,
                            name,
                            direction,
                        } => imp.add_port(id, name.as_str(), node_id, direction),
                        PipewireMessage::PortFormatChanged { id, media_type } => {
                            imp.port_media_type_changed(id, media_type)
                        }
                        PipewireMessage::LinkAdded {
                            id,
                            port_from,
                            port_to,
                            active,
                            media_type,
                        } => imp.add_link(id, port_from, port_to, active, media_type),
                        PipewireMessage::LinkStateChanged { id, active } => {
                            imp.link_state_changed(id, active)
                        }
                        PipewireMessage::LinkFormatChanged { id, media_type } => {
                            imp.link_format_changed(id, media_type)
                        }
                        PipewireMessage::NodeRemoved { id } => imp.remove_node(id),
                        PipewireMessage::PortRemoved { id, node_id } => {
                            imp.remove_port(id, node_id)
                        }
                        PipewireMessage::LinkRemoved { id } => imp.remove_link(id),
                        PipewireMessage::Connecting => {
                            if let Some(banner) = imp.connection_banner.borrow().as_ref() {
                                banner.set_revealed(true);
                            }
                        }
                        PipewireMessage::Connected => {
                            if let Some(banner) = imp.connection_banner.borrow().as_ref() {
                                banner.set_revealed(false);
                            }
                        }
                        PipewireMessage::Disconnected => {
                            imp.clear();
                        }
                    }
                }
            });
        }

        fn graph_view(&self) -> crate::ui::graph::GraphView {
            self.graph.borrow().clone().expect("graph should be set")
        }

        /// Add a new node to the view.
        fn add_node(&self, id: NodeId, name: &str, node_type: Option<NodeType>) {
            log::info!("Adding node to graph: id {}", id.0);

            let mut items = self.items.borrow_mut();
            if let Some(old_item) = items.get(&id.0) {
                if let Ok(old_node) = old_item.clone().dynamic_cast::<graph::Node>() {
                    self.graph_view().remove_node(&old_node);
                }
            }

            let node = graph::Node::new(name, id);

            items.insert(id.0, node.clone().upcast());

            self.graph_view().add_node(node, node_type);
        }

        /// Update a node tooltip to the view.
        fn node_name_changed(&self, id: NodeId, node_name: &str, media_name: &str) {
            let items = self.items.borrow();

            let Some(node) = items.get(&id.0) else {
                log::warn!("Node (id: {}) for changed name not found in graph manager", id.0);
                return;
            };
            let Some(node) = node.dynamic_cast_ref::<graph::Node>() else {
                log::warn!("Graph Manager item under node (id: {}) is not a node", id.0);
                return;
            };

            node.set_node_name(node_name);
            node.set_media_name(media_name);
        }

        /// Remove the node with the specified id from the view.
        fn remove_node(&self, id: NodeId) {
            log::info!("Removing node from graph: id {}", id.0);

            let Some(node) = self.items.borrow_mut().remove(&id.0) else {
                log::warn!("Unknown node (id={}) removed from graph", id.0);
                return;
            };
            let Ok(node) = node.dynamic_cast::<graph::Node>() else {
                log::warn!("Graph Manager item under node id {} is not a node", id.0);
                return;
            };

            self.graph_view().remove_node(&node);
        }

        /// Add a new port to the view.
        fn add_port(
            &self,
            id: PortId,
            name: &str,
            node_id: NodeId,
            direction: libspa::utils::Direction,
        ) {
            log::info!("Adding port to graph: id {}", id.0);

            let mut items = self.items.borrow_mut();

            if let Some(old_item) = items.get(&id.0) {
                if let Ok(old_port) = old_item.clone().dynamic_cast::<graph::Port>() {
                    if let Some(old_node_widget) = old_port.parent().and_downcast::<graph::Node>() {
                        old_node_widget.remove_port(&old_port);
                    }
                }
            }

            let Some(node) = items.get(&node_id.0) else {
                log::warn!(
                    "Node (id: {}) for port (id: {}) not found in graph manager",
                    node_id.0,
                    id.0
                );
                return;
            };
            let Ok(node) = node.clone().dynamic_cast::<graph::Node>() else {
                log::warn!("Graph Manager item under node id {} is not a node", node_id.0);
                return;
            };

            let port = graph::Port::new(id, name, direction);

            // Create or delete a link if the widget emits the "port-toggled" signal.
            port.connect_local(
                "port_toggled",
                false,
                glib::clone!(
                    #[weak(rename_to = app)]
                    self,
                    #[upgrade_or_default]
                    move |args| {
                        // Args always look like this: &[widget, id_port_from, id_port_to]
                        let port_from = PortId(args[1].get::<u32>().unwrap());
                        let port_to = PortId(args[2].get::<u32>().unwrap());

                        app.toggle_link(port_from, port_to);

                        None
                    }
                ),
            );

            items.insert(id.0, port.clone().upcast());

            node.add_port(port);
        }

        fn port_media_type_changed(&self, id: PortId, media_type: MediaType) {
            let items = self.items.borrow();

            let Some(port) = items.get(&id.0) else {
                log::warn!("Port (id: {}) for changed media type not found in graph manager", id.0);
                return;
            };
            let Some(port) = port.dynamic_cast_ref::<graph::Port>() else {
                log::warn!("Graph Manager item under port id {} is not a port", id.0);
                return;
            };

            port.set_media_type(media_type.as_raw())
        }

        /// Remove the port with the id `id` from the node with the id `node_id`
        /// from the view.
        fn remove_port(&self, id: PortId, node_id: NodeId) {
            log::info!("Removing port from graph: id {}, node_id: {}", id.0, node_id.0);

            let mut items = self.items.borrow_mut();

            let Some(node) = items.get(&node_id.0) else {
                log::warn!("Node (id: {}) for port (id: {}) not found in graph manager", node_id.0, id.0);
                return;
            };
            let Ok(node) = node.clone().dynamic_cast::<graph::Node>() else {
                log::warn!("Graph Manager item under node id {} is not a node", node_id.0);
                return;
            };
            let Some(port) = items.remove(&id.0) else {
                log::warn!("Unknown Port (id: {}) removed from graph", id.0);
                return;
            };
            let Ok(port) = port.dynamic_cast::<graph::Port>() else {
                log::warn!("Graph Manager item under port id {} is not a port", id.0);
                return;
            };

            node.remove_port(&port);
        }

        /// Add a new link to the view.
        fn add_link(
            &self,
            id: LinkId,
            output_port_id: PortId,
            input_port_id: PortId,
            active: bool,
            media_type: MediaType,
        ) {
            log::info!("Adding link to graph: id {}", id.0);

            let mut items = self.items.borrow_mut();

            if let Some(old_item) = items.get(&id.0) {
                if let Ok(old_link) = old_item.clone().dynamic_cast::<graph::Link>() {
                    self.graph
                        .borrow()
                        .as_ref()
                        .expect("graph should be set")
                        .remove_link(&old_link);
                }
            }

            let Some(output_port) = items.get(&output_port_id.0) else {
                log::warn!("Output port (id: {}) for link (id: {}) not found in graph manager", output_port_id.0, id.0);
                return;
            };
            let Ok(output_port) = output_port.clone().dynamic_cast::<graph::Port>() else {
                log::warn!("Graph Manager item under port id {} is not a port", output_port_id.0);
                return;
            };
            let Some(input_port) = items.get(&input_port_id.0) else {
                log::warn!("Output port (id: {}) for link (id: {}) not found in graph manager", input_port_id.0, id.0);
                return;
            };
            let Ok(input_port) = input_port.clone().dynamic_cast::<graph::Port>() else {
                log::warn!("Graph Manager item under port id {} is not a port", input_port_id.0);
                return;
            };

            let link = graph::Link::new();
            link.set_output_port(Some(&output_port));
            link.set_input_port(Some(&input_port));
            link.set_active(active);
            link.set_media_type(media_type);

            items.insert(id.0, link.clone().upcast());

            // Update graph to contain the new link.
            self.graph
                .borrow()
                .as_ref()
                .expect("graph should be set")
                .add_link(link);
        }

        fn link_state_changed(&self, id: LinkId, active: bool) {
            log::info!(
                "Link state changed: Link (id={}) is now {}",
                id.0,
                if active { "active" } else { "inactive" }
            );

            let items = self.items.borrow();

            let Some(link) = items.get(&id.0) else {
                log::warn!("Link state changed on unknown link (id={})", id.0);
                return;
            };
            let Some(link) = link.dynamic_cast_ref::<graph::Link>() else {
                log::warn!("Graph Manager item under link id {} is not a link", id.0);
                return;
            };

            link.set_active(active);
        }

        fn link_format_changed(&self, id: LinkId, media_type: libspa::param::format::MediaType) {
            let items = self.items.borrow();

            let Some(link) = items.get(&id.0) else {
                log::warn!("Link (id: {}) for changed media type not found in graph manager", id.0);
                return;
            };
            let Some(link) = link.dynamic_cast_ref::<graph::Link>() else {
                log::warn!("Graph Manager item under link id {} is not a link", id.0);
                return;
            };
            link.set_media_type(media_type);
        }

        // Toggle a link between the two specified ports on the remote pipewire server.
        fn toggle_link(&self, port_from: PortId, port_to: PortId) {
            let sender = self.pw_sender.get().expect("pw_sender shoud be set");
            sender
                .send(crate::GtkMessage::ToggleLink { port_from, port_to })
                .expect("Failed to send message");
        }

        /// Remove the link with the specified id from the view.
        fn remove_link(&self, id: LinkId) {
            log::info!("Removing link from graph: id {}", id.0);

            let Some(link) = self.items.borrow_mut().remove(&id.0) else {
                log::warn!("Unknown Link (id={}) removed from graph", id.0);
                return;
            };
            let Ok(link) = link.dynamic_cast::<graph::Link>() else {
                log::warn!("Graph Manager item under link id {} is not a link", id.0);
                return;
            };

            self.graph_view().remove_link(&link);
        }

        fn clear(&self) {
            self.items.borrow_mut().clear();
            self.graph_view().clear();
        }
    }
}

glib::wrapper! {
    pub struct GraphManager(ObjectSubclass<imp::GraphManager>);
}

impl GraphManager {
    pub fn new(
        graph: &GraphView,
        connection_banner: &adw::Banner,
        sender: PwSender<GtkMessage>,
        receiver: async_channel::Receiver<PipewireMessage>,
    ) -> Self {
        let res: Self = glib::Object::builder()
            .property("graph", graph)
            .property("connection-banner", connection_banner)
            .build();

        res.imp().attach_receiver(receiver);
        assert!(
            res.imp().pw_sender.set(sender).is_ok(),
            "Should be able to set pw_sender)"
        );

        res
    }
}

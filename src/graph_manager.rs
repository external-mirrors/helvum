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
    use petgraph::visit::IntoEdgeReferences;

    use std::{cell::Cell, cell::OnceCell, cell::RefCell, collections::HashMap, collections::HashSet};

    use crate::{
        ui::graph, GtkMessage, LinkId, MediaType, NodeId, NodeType,
        PipewireMessage, PortId,
        preset_manager,
    };

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::GraphManager)]
    pub struct GraphManager {
        #[property(get, set, construct_only, nullable)]
        pub graph: RefCell<Option<crate::ui::graph::GraphView>>,

        #[property(get, set, construct_only, nullable)]
        pub matrix_view: RefCell<Option<crate::ui::MatrixView>>,

        #[property(get, set, construct_only, nullable)]
        pub connection_banner: RefCell<Option<adw::Banner>>,

        pub pw_sender: OnceCell<PwSender<crate::GtkMessage>>,
        pub items: RefCell<HashMap<u32, glib::Object>>,
        /// Nodes that have been removed and are waiting to be revived.
        pub(super) offline_nodes: RefCell<HashMap<String, graph::Node>>,
        pub(super) app_groups: RefCell<HashMap<String, graph::Node>>,
        /// IDs that are currently detached and should not be grouped.
        pub(super) detached_ids: RefCell<HashSet<u32>>,
        pub(super) pending_links: RefCell<HashSet<preset_manager::ConnectionPreset>>,
        /// Links that were manually deleted by the user and should not be ghosted.
        pub(super) pending_deletions: RefCell<HashSet<String>>,

        pub(super) master_links: RefCell<HashMap<String, graph::Link>>,
        pub(super) real_to_master: RefCell<HashMap<u32, String>>,
        pub(super) link_to_internal_node: RefCell<HashMap<u32, Vec<u32>>>,
        pub(super) matrix_update_pending: Cell<bool>,
    }

    impl GraphManager {
        pub(super) fn queue_matrix_update(&self) {
            if !self.matrix_update_pending.get() {
                self.matrix_update_pending.set(true);
                glib::source::idle_add_local_once(glib::clone!(
                    #[weak(rename_to = manager)]
                    self,
                    move || {
                        manager.matrix_update_pending.set(false);
                        if let Some(matrix) = manager.matrix_view.borrow().as_ref() {
                            if matrix.is_mapped() {
                                if let Some(sender) = manager.pw_sender.get() {
                                    matrix.update_view(&manager.items.borrow(), &manager.master_links.borrow(), sender.clone());
                                }
                            }
                        }
                    }
                ));
            }
        }
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
                            internal_name,
                            app_name,
                            node_type,
                        } => imp.add_node(id, name.as_str(), internal_name.as_str(), app_name.as_str(), node_type, false),
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
                    imp.queue_matrix_update();
                }
            });
        }

        fn graph_view(&self) -> crate::ui::graph::GraphView {
            self.graph.borrow().clone().expect("graph should be set")
        }

        /// Add a new node to the view.
        fn add_node(&self, id: NodeId, name: &str, internal_name: &str, app_name: &str, node_type: Option<NodeType>, is_detached: bool) {
            let is_detached = is_detached || self.detached_ids.borrow().contains(&id.0);
            
            let existing_node = {
                let items = self.items.borrow_mut();
                let offline_nodes = self.offline_nodes.borrow_mut();
                let mut app_groups = self.app_groups.borrow_mut();

                let mut node_to_remove = None;
                if let Some(old_item) = items.get(&id.0) {
                    if let Ok(old_node) = old_item.clone().dynamic_cast::<graph::Node>() {
                        old_node.remove_sub_node(id.0);
                        if old_node.sub_node_count() == 0 {
                            node_to_remove = Some(old_node);
                        } else {
                            let ports_to_move: Vec<_> = old_node.ports().into_iter().filter(|p| p.node_id() == id.0).collect();
                            for port in ports_to_move {
                                old_node.remove_port(&port);
                            }
                        }
                    }
                }

                if let Some(old_node) = node_to_remove {
                    app_groups.remove(&old_node.app_name());
                    drop(items);
                    drop(offline_nodes);
                    drop(app_groups);
                    self.graph_view().remove_node(&old_node);
                } else {
                    drop(items);
                    drop(offline_nodes);
                    drop(app_groups);
                }

                let is_app = !app_name.is_empty();

                if is_app && !is_detached {
                    let mut app_groups = self.app_groups.borrow_mut();
                    if let Some(group_node) = app_groups.get(app_name) {
                        log::info!("Adding stream {} to existing group {}", name, app_name);
                        group_node.add_sub_node(id.0, name);
                        group_node.set_online(true);
                        self.items.borrow_mut().insert(id.0, group_node.clone().upcast());
                        self.graph_view().queue_draw();
                        return;
                    }
                    
                    let mut offline_nodes = self.offline_nodes.borrow_mut();
                    if let Some(node) = offline_nodes.remove(app_name) {
                        log::info!("Reviving application group: {}", app_name);
                        node.set_online(true);
                        node.add_sub_node(id.0, name);
                        app_groups.insert(app_name.to_string(), node.clone());
                        self.items.borrow_mut().insert(id.0, node.clone().upcast());

                        let ports = node.ports();
                        for port in ports {
                            let port_name = port.name();
                            let direction = libspa::utils::Direction::from(port.port_direction());
                            let port_id = port.pw_id();
                            self.check_and_create_master_port(&node, &port_name, direction, port_id);
                        }

                        self.graph_view().queue_draw();
                        return;
                    }
                }
                
                let mut app_groups = self.app_groups.borrow_mut();
                let mut items = self.items.borrow_mut();
                
                log::info!("Adding new node: {} ({}) for app {}", name, internal_name, app_name);
                let display_name = if is_app && !is_detached { app_name } else { name };
                let node = graph::Node::new(display_name, id);
                node.set_media_name(internal_name.to_string());
                node.set_app_name(app_name.to_string());
                node.set_node_type(node_type);
                node.add_sub_node(id.0, name);
                node.set_online(true);
                node.set_is_detached(is_detached);
                node.set_is_app(!app_name.is_empty());

                if is_app && !is_detached {
                    app_groups.insert(app_name.to_string(), node.clone());
                }
                items.insert(id.0, node.clone().upcast());
                Some(node)
            };

            if let Some(node) = existing_node {
                let self_obj = self.obj().clone();
                node.connect_local("detach", false, move |args| {
                    let id = args[1].get::<u32>().unwrap();
                    self_obj.imp().detach_node(id);
                    None
                });

                let self_obj = self.obj().clone();
                node.connect_local("reattach", false, move |args| {
                    let id = args[1].get::<u32>().unwrap();
                    self_obj.imp().reattach_node(id);
                    None
                });

                let self_obj = self.obj().clone();
                node.connect_local("delete", false, move |args| {
                    let node = args[0].get::<graph::Node>().unwrap();
                    self_obj.imp().delete_offline_node(&node);
                    None
                });

                self.graph_view().add_node(node, node_type);
                self.graph_view().queue_draw();
            }
        }

        fn detach_node(&self, id: u32) {
            log::info!("Detaching node {}", id);
            
            let mut detached_ids = self.detached_ids.borrow_mut();
            detached_ids.insert(id);
            
            let items = self.items.borrow();
            let Some(item) = items.get(&id) else { return };
            let Ok(node) = item.clone().dynamic_cast::<graph::Node>() else { return };
            
            let name = node.sub_nodes().get(&id).cloned().unwrap_or_else(|| node.node_name());
            let internal_name = node.media_name();
            let app_name = node.app_name();
            let node_type = node.node_type();
            
            drop(items);
            drop(detached_ids);

            self.add_node(NodeId(id), &name, &internal_name, &app_name, node_type, true);
        }

        fn reattach_node(&self, id: u32) {
            log::info!("Reattaching node {}", id);
            self.detached_ids.borrow_mut().remove(&id);
            
            let items = self.items.borrow();
            let Some(item) = items.get(&id) else { return };
            let Ok(node) = item.clone().dynamic_cast::<graph::Node>() else { return };
            
            let name = node.sub_nodes().get(&id).cloned().unwrap_or_else(|| node.node_name());
            let internal_name = node.media_name();
            let app_name = node.app_name();
            let node_type = node.node_type();
            
            drop(items);
            self.add_node(NodeId(id), &name, &internal_name, &app_name, node_type, false);
        }

        fn delete_offline_node(&self, node: &graph::Node) {
            log::info!("Deleting offline node {}", node.app_name());
            self.graph_view().remove_node(node);
            self.offline_nodes.borrow_mut().remove(&node.app_name());
        }

        pub fn save_preset(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
            let mut preset = preset_manager::Preset::default();
            let items = self.items.borrow();
            let detached_ids = self.detached_ids.borrow();

            for item in items.values() {
                let Ok(link) = item.clone().dynamic_cast::<graph::Link>() else { continue };
                
                let Some(source_port) = link.imp().output_port.upgrade() else { continue };
                let Some(sink_port) = link.imp().input_port.upgrade() else { continue };
                
                let source_node_id = source_port.node_id();
                let sink_node_id = sink_port.node_id();
                
                if detached_ids.contains(&source_node_id) || detached_ids.contains(&sink_node_id) {
                    continue;
                }

                let Some(source_node_item) = items.get(&source_node_id) else { continue };
                let Some(sink_node_item) = items.get(&sink_node_id) else { continue };
                
                let Ok(source_node) = source_node_item.clone().dynamic_cast::<graph::Node>() else { continue };
                let Ok(sink_node) = sink_node_item.clone().dynamic_cast::<graph::Node>() else { continue };

                preset.connections.push(preset_manager::ConnectionPreset {
                    source_app: source_node.app_name(),
                    source_port: source_port.name(),
                    sink_app: sink_node.app_name(),
                    sink_port: sink_port.name(),
                });
            }

            preset.save_to_file(path)
        }

        pub fn load_preset(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
            let preset = preset_manager::Preset::load_from_file(path)?;
            
            let mut pending = self.pending_links.borrow_mut();
            for conn in preset.connections {
                log::info!("Queuing preset link: {}/{} -> {}/{}", conn.source_app, conn.source_port, conn.sink_app, conn.sink_port);
                pending.insert(conn);
            }
            
            drop(pending);
            self.check_pending_links();
            
            Ok(())
        }

        fn check_pending_links(&self) {
            let pending = self.pending_links.borrow().clone();
            let items = self.items.borrow();

            for conn in pending {
                let source_port = items.values().find_map(|item| {
                    let port = item.clone().dynamic_cast::<graph::Port>().ok()?;
                    if port.port_direction() == graph::PortDirection::Output && port.name() == conn.source_port {
                        let node = items.get(&port.node_id())?.clone().dynamic_cast::<graph::Node>().ok()?;
                        if node.app_name() == conn.source_app {
                            return Some(port);
                        }
                    }
                    None
                });

                let sink_port = items.values().find_map(|item| {
                    let port = item.clone().dynamic_cast::<graph::Port>().ok()?;
                    if port.port_direction() == graph::PortDirection::Input && port.name() == conn.sink_port {
                        let node = items.get(&port.node_id())?.clone().dynamic_cast::<graph::Node>().ok()?;
                        if node.app_name() == conn.sink_app {
                            return Some(port);
                        }
                    }
                    None
                });

                if let (Some(source), Some(sink)) = (source_port, sink_port) {
                    log::info!("Applying preset link: {}/{} -> {}/{}", conn.source_app, conn.source_port, conn.sink_app, conn.sink_port);
                    self.pw_sender.get().unwrap()
                        .send(GtkMessage::EnsureLink {
                            port_from: source.pw_id(),
                            port_to: sink.pw_id(),
                        })
                        .expect("Failed to send message");
                    
                    self.pending_links.borrow_mut().remove(&conn);
                }
            }
        }


        /// Update a node tooltip to the view.
        fn node_name_changed(&self, id: NodeId, node_name: &str, media_name: &str) {
            let items = self.items.borrow();

            let Some(node) = items.get(&id.0) else {
                log::warn!(
                    "Node (id: {}) for changed name not found in graph manager",
                    id.0
                );
                return;
            };
            let Some(node) = node.dynamic_cast_ref::<graph::Node>() else {
                log::warn!("Graph Manager item under node (id: {}) is not a node", id.0);
                return;
            };

            node.set_node_name(node_name);
            node.set_media_name(media_name.to_string());
        }

        /// Remove a node from the view.
        fn remove_node(&self, id: NodeId) {
                let node_to_ghost: Option<graph::Node>;

            {
                let mut items = self.items.borrow_mut();
                let mut offline_nodes = self.offline_nodes.borrow_mut();
                let mut app_groups = self.app_groups.borrow_mut();

                let Some(item) = items.remove(&id.0) else {
                    log::warn!("Node (id: {}) for removal not found in graph manager", id.0);
                    return;
                };

                let Ok(node) = item.clone().dynamic_cast::<graph::Node>() else {
                    log::warn!("Graph Manager item under node id {} is not a node", id.0);
                    return;
                };

                node.remove_sub_node(id.0);
                
                // Handle ports of the sub-node
                let sub_node_ports: Vec<_> = node.ports().into_iter().filter(|p| p.node_id() == id.0).collect();
                if node.auto_delete() {
                    for p in sub_node_ports {
                        node.remove_port(&p);
                    }
                } else {
                    for p in sub_node_ports {
                        p.set_online(false);
                    }
                }

                if node.sub_node_count() == 0 {
                    let app_name = node.app_name();
                    if node.auto_delete() {
                        log::info!("Application group {} fully offline and auto-delete enabled, removing", app_name);
                        app_groups.remove(&app_name);
                        drop(items);
                        drop(offline_nodes);
                        drop(app_groups);
                        self.graph_view().remove_node(&node);
                        return;
                    }
                    log::info!("Application group {} went fully offline, ghosting", app_name);
                    app_groups.remove(&app_name);
                    offline_nodes.insert(app_name, node.clone());
                    node_to_ghost = Some(node);
                } else {
                    log::info!("Sub-node {} removed from group {}, group stays online", id.0, node.app_name());
                    node_to_ghost = None;
                }
            }
            
            if let Some(node) = node_to_ghost {
                node.set_online(false);
                for port in node.ports() {
                    port.set_online(false);
                }
            }

            self.graph_view().queue_draw();
        }

        /// Add a new port to the view.
        fn add_port(
            &self,
            id: PortId,
            name: &str,
            node_id: NodeId,
            direction: libspa::utils::Direction,
        ) {
            log::info!("Adding port to graph: id {}, node_id {}", id.0, node_id.0);

            let port = graph::Port::new(id, name, direction);
            port.set_node_id(node_id.0);

            let mut port_added = false;
            {
                let mut items = self.items.borrow_mut();

                if let Some(old_item) = items.get(&id.0) {
                    if let Ok(old_port) = old_item.clone().dynamic_cast::<graph::Port>() {
                        if let Some(node) = items.get(&old_port.node_id()) {
                            if let Ok(node) = node.clone().dynamic_cast::<graph::Node>() {
                                node.remove_port(&old_port);
                            }
                        }
                    }
                }

                if let Some(node_item) = items.get(&node_id.0) {
                    if let Ok(node) = node_item.clone().dynamic_cast::<graph::Node>() {
                        node.add_port(port.clone());
                        items.insert(id.0, port.clone().upcast());
                        port_added = true;

                        let self_obj = self.obj().clone();
                        port.connect_local(
                            "port-toggled",
                            false,
                            glib::clone!(
                                #[weak(rename_to = app)]
                                self_obj,
                                #[upgrade_or_default]
                                move |args| {
                                    let port_from = PortId(args[1].get::<u32>().unwrap());
                                    let port_to = PortId(args[2].get::<u32>().unwrap());

                                    app.imp().toggle_link(port_from, port_to);

                                    None
                                }
                            ),
                        );
                    }
                }
            }

            if port_added {
                let node = {
                    let items = self.items.borrow();
                    items.get(&node_id.0).and_then(|i| i.clone().dynamic_cast::<graph::Node>().ok())
                };
                if let Some(node) = node {
                    self.check_and_create_master_port(&node, &name, direction, id);
                }

                {
                    let items = self.items.borrow();
                    if let Some(port) = items.get(&id.0).and_then(|i| i.clone().dynamic_cast::<graph::Port>().ok()) {
                        self.restore_links_for_port(&port);
                    }
                }
                self.check_pending_links();
                self.graph_view().queue_draw();
            }
        }

        fn port_media_type_changed(&self, id: PortId, media_type: MediaType) {
            let (port, master_id) = {
                let items = self.items.borrow();
                let port = items.get(&id.0).and_then(|p| p.clone().dynamic_cast::<graph::Port>().ok());
                let master_id = self.get_master_port_id_with_items(&items, id);
                (port, master_id)
            };

            if let Some(port) = port {
                port.set_media_type(media_type);
                
                // Also update master port if it exists
                if let Some(master_id) = master_id {
                    let items = self.items.borrow();
                    if let Some(master) = items.get(&master_id.0).and_then(|i| i.dynamic_cast_ref::<graph::Port>()) {
                        master.set_media_type(media_type);
                    }
                }
            }
        }

        /// Remove the port with the id `id` from the node with the id `node_id`
        /// from the view.
        fn remove_port(&self, id: PortId, node_id: NodeId) {
            log::info!(
                "Removing port from graph: id {}, node_id: {}",
                id.0,
                node_id.0
            );

            let mut items = self.items.borrow_mut();

            let Some(item) = items.remove(&id.0) else {
                log::warn!("Unknown Port (id: {}) removed from graph", id.0);
                return;
            };
            let Ok(port) = item.dynamic_cast::<graph::Port>() else {
                log::warn!("Graph Manager item under port id {} is not a port", id.0);
                return;
            };

            port.set_online(false);
            self.graph_view().queue_draw();
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
            log::info!("Adding link to graph: id {} ({} -> {})", id.0, output_port_id.0, input_port_id.0);

            let out_node = self.get_node_for_port_id(output_port_id);
            let in_node = self.get_node_for_port_id(input_port_id);
            
            if let (Some(out_n), Some(in_n)) = (out_node.clone(), in_node.clone()) {
                if out_n == in_n && out_n.is_app() {
                    log::info!("Detected internal link for node {}, routing to sub-canvas", out_n.pw_name());
                    out_n.add_internal_link(id.0, output_port_id.0, input_port_id.0, active, media_type);
                    self.link_to_internal_node.borrow_mut().insert(id.0, vec![out_n.pw_id().0]);
                    return;
                }
            }

            let master_out_id = self.get_master_port_id(output_port_id);
            let master_in_id = self.get_master_port_id(input_port_id);
            let effective_out_id = master_out_id.unwrap_or(output_port_id);
            let effective_in_id = master_in_id.unwrap_or(input_port_id);

            if effective_out_id != output_port_id || effective_in_id != input_port_id {
                let key = format!("{}-{}", effective_out_id.0, effective_in_id.0);
                self.real_to_master.borrow_mut().insert(id.0, key.clone());

                let mut internal_nodes = Vec::new();
                if let Some(out_n) = out_node {
                    if out_n.is_app() {
                        out_n.add_internal_link(id.0, output_port_id.0, effective_out_id.0, active, media_type);
                        internal_nodes.push(out_n.pw_id().0);
                    }
                }
                if let Some(in_n) = in_node {
                    if in_n.is_app() {
                        in_n.add_internal_link(id.0, effective_in_id.0, input_port_id.0, active, media_type);
                        internal_nodes.push(in_n.pw_id().0);
                    }
                }
                self.link_to_internal_node.borrow_mut().insert(id.0, internal_nodes);

                if let Some(master_link) = self.master_links.borrow().get(&key) {
                    master_link.set_active(true);
                    return;
                }

                let items = self.items.borrow();
                let m_out_port = items.get(&effective_out_id.0).unwrap().clone().downcast::<graph::Port>().unwrap();
                let m_in_port = items.get(&effective_in_id.0).unwrap().clone().downcast::<graph::Port>().unwrap();

                let master_link = graph::Link::new();
                master_link.set_output_port(Some(&m_out_port));
                master_link.set_input_port(Some(&m_in_port));
                master_link.set_active(active);
                master_link.set_online(true);
                master_link.set_media_type(media_type);

                self.master_links.borrow_mut().insert(key, master_link.clone());
                self.graph_view().add_link(master_link);
                self.graph_view().queue_draw();
                return;
            }

            let mut items = self.items.borrow_mut();

            if let Some(old_item) = items.get(&id.0) {
                if let Ok(old_link) = old_item.clone().dynamic_cast::<graph::Link>() {
                    self.graph_view().remove_link(&old_link);
                }
            }

            let Some(output_port) = items.get(&output_port_id.0) else {
                log::warn!(
                    "Output port (id: {}) for link (id: {}) not found in graph manager",
                    output_port_id.0,
                    id.0
                );
                return;
            };
            let Ok(output_port) = output_port.clone().dynamic_cast::<graph::Port>() else {
                log::warn!(
                    "Graph Manager item under port id {} is not a port",
                    output_port_id.0
                );
                return;
            };
            let Some(input_port) = items.get(&input_port_id.0) else {
                log::warn!(
                    "Input port (id: {}) for link (id: {}) not found in graph manager",
                    input_port_id.0,
                    id.0
                );
                return;
            };
            let Ok(input_port) = input_port.clone().dynamic_cast::<graph::Port>() else {
                log::warn!(
                    "Graph Manager item under port id {} is not a port",
                    input_port_id.0
                );
                return;
            };

            let link = {
                let graph_view = self.graph_view();
                let graph = graph_view.graph().borrow();
                let found = graph.edge_weights().find(|l| {
                    l.output_port().as_ref() == Some(&output_port) && l.input_port().as_ref() == Some(&input_port)
                }).cloned();
                found
            };

            if let Some(link) = link {
                log::info!("Reviving offline link: id {}", id.0);
                link.set_active(active);
                link.set_online(true);
                link.set_media_type(media_type);
                items.insert(id.0, link.upcast());
                self.graph_view().queue_draw();
                return;
            }

            let link = graph::Link::new();
            link.set_output_port(Some(&output_port));
            link.set_input_port(Some(&input_port));
            link.set_active(active);
            link.set_online(true);
            link.set_media_type(media_type);

            items.insert(id.0, link.clone().upcast());

            // Update graph to contain the new link.
            self.graph_view().add_link(link);
            self.graph_view().queue_draw();
        }

        fn link_state_changed(&self, id: LinkId, active: bool) {
            log::info!(
                "Link state changed: Link (id={}) is now {}",
                id.0,
                if active { "active" } else { "inactive" }
            );

            let node_ids_opt = self.link_to_internal_node.borrow().get(&id.0).cloned();
            if let Some(node_ids) = node_ids_opt {
                let items = self.items.borrow();
                for node_id in node_ids {
                    if let Some(node) = items.get(&node_id).and_then(|i| i.dynamic_cast_ref::<graph::Node>()) {
                        node.update_internal_link_state(id.0, active);
                    }
                }
                if self.real_to_master.borrow().get(&id.0).is_none() {
                    return;
                }
            }

            let key_opt = self.real_to_master.borrow().get(&id.0).cloned();
            if let Some(key) = key_opt {
                if let Some(master) = self.master_links.borrow().get(&key) {
                    if active {
                        master.set_active(true);
                    } else {
                        let real_to_master = self.real_to_master.borrow();
                        let any_active = real_to_master.iter().filter(|(_, k)| **k == key).any(|(rid, _)| {
                            if *rid == id.0 { return false; }
                            self.items.borrow().get(rid)
                                .and_then(|i| i.dynamic_cast_ref::<graph::Link>())
                                .map(|l| l.active())
                                .unwrap_or(false)
                        });
                        if !any_active {
                            master.set_active(false);
                        }
                    }
                }
            }
            let items = self.items.borrow();

            let Some(link) = items.get(&id.0) else {
                return;
            };
            let Some(link) = link.dynamic_cast_ref::<graph::Link>() else {
                return;
            };

            link.set_active(active);
            self.graph_view().queue_draw();
        }

        fn link_format_changed(&self, id: LinkId, media_type: libspa::param::format::MediaType) {
            let node_ids_opt = self.link_to_internal_node.borrow().get(&id.0).cloned();
            if let Some(node_ids) = node_ids_opt {
                let items = self.items.borrow();
                for node_id in node_ids {
                    if let Some(node) = items.get(&node_id).and_then(|i| i.dynamic_cast_ref::<graph::Node>()) {
                        node.update_internal_link_format(id.0, media_type);
                    }
                }
                if self.real_to_master.borrow().get(&id.0).is_none() {
                    return;
                }
            }

            let key_opt = self.real_to_master.borrow().get(&id.0).cloned();
            if let Some(key) = key_opt {
                if let Some(master) = self.master_links.borrow().get(&key) {
                    master.set_media_type(media_type);
                }
            }

            let items = self.items.borrow();

            let Some(link) = items.get(&id.0) else {
                return;
            };
            let Some(link) = link.dynamic_cast_ref::<graph::Link>() else {
                return;
            };
            link.set_media_type(media_type);
            self.graph_view().queue_draw();
        }

        // Toggle a link between the two specified ports on the remote pipewire server.
        // If it's a ghost link, we remove it from the graph.
        // If it's an active link (or no link exists), we ask PipeWire to toggle it.
        fn toggle_link(&self, port_from: PortId, port_to: PortId) {
            log::info!("Toggling link between {} and {}", port_from.0, port_to.0);
            let mut is_proxy = false;
            let mut proxy_port_id = PortId(0);
            let mut other_port_id = PortId(0);
            let mut proxy_is_from = false;

            if port_from.0 >= 0xF0000000 {
                is_proxy = true;
                proxy_port_id = port_from;
                other_port_id = port_to;
                proxy_is_from = true;
            } else if port_to.0 >= 0xF0000000 {
                is_proxy = true;
                proxy_port_id = port_to;
                other_port_id = port_from;
                proxy_is_from = false;
            }

            if is_proxy {
                log::info!("Proxy port detected, replicating link to all sub-ports");
                let items = self.items.borrow();
                for item in items.values() {
                    if let Ok(node) = item.clone().dynamic_cast::<graph::Node>() {
                        if node.is_detached() { continue; }
                        if let Some(master) = node.master_ports().into_iter().find(|p| p.pw_id() == proxy_port_id) {
                            let port_name = master.name();
                            let direction = master.port_direction();
                            
                            let ports = node.ports();
                            for port in ports {
                                if port.name() == port_name && port.port_direction() == direction {
                                    let sub_from = if proxy_is_from { port.pw_id() } else { other_port_id };
                                    let sub_to = if proxy_is_from { other_port_id } else { port.pw_id() };
                                    self.toggle_link_internal(sub_from, sub_to);
                                }
                            }
                            return;
                        }
                    }
                }
                return;
            }

            self.toggle_link_internal(port_from, port_to);
        }

        fn toggle_link_internal(&self, port_from: PortId, port_to: PortId) {
            // If there's a ghost link between these ports, remove it from the graph instead of toggling PipeWire.
            let items = self.items.borrow();
            let port_from_widget = items.get(&port_from.0).and_then(|i| i.dynamic_cast_ref::<graph::Port>());
            let port_to_widget = items.get(&port_to.0).and_then(|i| i.dynamic_cast_ref::<graph::Port>());

            if let (Some(p1), Some(p2)) = (port_from_widget, port_to_widget) {
                let graph_view = self.graph_view();
                let graph = graph_view.graph().borrow();
                
                // Find any link between these ports
                let found_link = graph.edge_weights().find(|l| {
                    (l.output_port().as_ref() == Some(p1) && l.input_port().as_ref() == Some(p2)) ||
                    (l.output_port().as_ref() == Some(p2) && l.input_port().as_ref() == Some(p1))
                }).cloned();

                if let Some(link) = found_link {
                    if !link.online() {
                        log::info!("Removing ghost link from graph (manual delete)");
                        graph_view.remove_link(&link);
                        graph_view.queue_draw();
                        return;
                    } else {
                        // It's an active link, mark it for permanent removal when PipeWire confirms
                        log::info!("Marking active link for permanent removal: {} -> {}", port_from.0, port_to.0);
                        let key = if port_from.0 < port_to.0 {
                            format!("{}-{}", port_from.0, port_to.0)
                        } else {
                            format!("{}-{}", port_to.0, port_from.0)
                        };
                        self.pending_deletions.borrow_mut().insert(key);
                    }
                }
            }

            self.request_link_toggle(port_from, port_to);
        }

        fn request_link_toggle(&self, port_from: PortId, port_to: PortId) {
            let sender = self.pw_sender.get().expect("pw_sender shoud be set");
            sender
                .send(crate::GtkMessage::ToggleLink { port_from, port_to })
                .expect("Failed to send message");
        }

        fn request_link_ensure(&self, port_from: PortId, port_to: PortId) {
            let sender = self.pw_sender.get().expect("pw_sender shoud be set");
            sender
                .send(crate::GtkMessage::EnsureLink { port_from, port_to })
                .expect("Failed to send message");
        }

        // Remove the link with the specified id from the view.
        fn remove_link(&self, id: LinkId) {
            log::info!("Removing link from graph: id {}", id.0);

            let node_ids_opt = self.link_to_internal_node.borrow_mut().remove(&id.0);
            if let Some(node_ids) = node_ids_opt {
                let items = self.items.borrow();
                for node_id in node_ids {
                    if let Some(node) = items.get(&node_id).and_then(|i| i.dynamic_cast_ref::<graph::Node>()) {
                        node.remove_internal_link(id.0);
                    }
                }
                if self.real_to_master.borrow().get(&id.0).is_none() {
                    return;
                }
            }
            let key_opt = self.real_to_master.borrow_mut().remove(&id.0);
            if let Some(key) = key_opt {
                let still_exists = self.real_to_master.borrow().values().any(|v| v == &key);
                if !still_exists {
                    if let Some(link) = self.master_links.borrow_mut().remove(&key) {
                        log::info!("Removing last real link for master key {}, deleting master link widget", key);
                        self.graph_view().remove_link(&link);
                    }
                }
                return;
            }

            let mut items = self.items.borrow_mut();

            let Some(item) = items.remove(&id.0) else {
                log::warn!("Unknown Link (id={}) removed from graph", id.0);
                return;
            };
            let Ok(link) = item.dynamic_cast::<graph::Link>() else {
                log::warn!("Graph Manager item under link id {} is not a link", id.0);
                return;
            };

            // Check if this removal was initiated by the user in Helvum
            let is_pending = if let (Some(out_p), Some(in_p)) = (link.output_port(), link.input_port()) {
                let id1 = out_p.pw_id().0;
                let id2 = in_p.pw_id().0;
                let key = if id1 < id2 {
                    format!("{}-{}", id1, id2)
                } else {
                    format!("{}-{}", id2, id1)
                };
                self.pending_deletions.borrow_mut().remove(&key)
            } else {
                false
            };

            if is_pending {
                log::info!("Link removed manually by user (Helvum), deleting from graph (id={})", id.0);
                self.graph_view().remove_link(&link);
            } else {
                let output_online = link.output_port().map(|p| p.online()).unwrap_or(false);
                let input_online = link.input_port().map(|p| p.online()).unwrap_or(false);

                if output_online && input_online {
                    log::info!("Link removed while ports are online, deferring removal check (id={})", id.0);
                    link.set_online(false);
                    link.set_pending_check(true);
                    
                    let obj = self.obj().clone();
                    let link_clone = link.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                        let imp = obj.imp();
                        let out_p = link_clone.output_port();
                        let in_p = link_clone.input_port();
                        
                        let still_online = if let (Some(out_p), Some(in_p)) = (out_p, in_p) {
                            out_p.online() && in_p.online()
                        } else {
                            false
                        };

                        if still_online {
                            log::info!("Ports still online after 100ms, removing link permanently");
                            imp.graph_view().remove_link(&link_clone);
                            imp.graph_view().queue_draw();
                        } else {
                            log::info!("One port went offline, keeping link as ghost");
                            link_clone.set_pending_check(false);
                            imp.graph_view().queue_draw();
                        }
                        glib::ControlFlow::Break
                    });
                } else {
                    log::info!("Link removed automatically (node offline), graying out (id={})", id.0);
                    link.set_online(false);
                }
            }
            self.graph_view().queue_draw();
        }

        fn restore_links_for_port(&self, port: &graph::Port) {
            let graph_view = self.graph_view();
            let graph = graph_view.graph().borrow();

            for edge in graph.edge_indices() {
                let link = &graph[edge];
                let is_related = link.output_port().as_ref() == Some(port) || link.input_port().as_ref() == Some(port);
                
                if is_related && !link.online() {
                    let out_p = link.output_port();
                    let in_p = link.input_port();
                    if let (Some(out_p), Some(in_p)) = (out_p, in_p) {
                        log::info!("Restoring persistent link in PipeWire (ensure): {} -> {}", out_p.pw_id(), in_p.pw_id());
                        self.request_link_ensure(out_p.pw_id(), in_p.pw_id());
                    }
                }
            }
        }


        fn check_and_create_master_port(&self, node: &graph::Node, name: &str, direction: libspa::utils::Direction, id: PortId) {
            if !node.is_app() || node.is_detached() { return; }

            let n = name.to_lowercase();
            let is_common = n.contains("front") || n.contains("capture") || n.contains("playback") || 
                            n.contains("monitor") || n.contains("input") || n.contains("output") ||
                            n.contains("aux") || n.contains("fl") || n.contains("fr") || n.contains("rl") || n.contains("rr") ||
                            n.contains("fc") || n.contains("lfe") || n.contains("sl") || n.contains("sr") ||
                            ["l", "r"].contains(&n.as_str());

            if is_common {
                if !node.has_master_port(name) {
                    log::info!("Creating master port proxy for {}", name);
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    node.app_name().hash(&mut hasher);
                    name.hash(&mut hasher);
                    let hash = hasher.finish() as u32;
                    let proxy_port_id = PortId(0xF0000000 | (hash & 0x0FFFFFFF));
                    
                    let proxy = graph::Port::new(proxy_port_id, name, direction);
                    
                    let items = self.items.borrow();
                    if let Some(real_port) = items.get(&id.0).and_then(|p| p.dynamic_cast_ref::<graph::Port>()) {
                        proxy.set_media_type(real_port.media_type().into());
                    }
                    drop(items);

                    proxy.set_node_id(node.sub_nodes().keys().next().cloned().unwrap_or(0));
                    
                    node.add_master_port(proxy.clone());
                    self.items.borrow_mut().insert(proxy_port_id.0, proxy.clone().upcast());
                    
                    let self_obj = self.obj().clone();
                    proxy.connect_local(
                        "port-toggled",
                        false,
                        glib::clone!(
                            #[weak(rename_to = app)]
                            self_obj,
                            #[upgrade_or_default]
                            move |args| {
                                let port_from = PortId(args[1].get::<u32>().unwrap());
                                let port_to = PortId(args[2].get::<u32>().unwrap());
                                app.imp().toggle_link(port_from, port_to);
                                None
                            }
                        ),
                    );
                }
                
                if let Some(master) = node.master_ports().into_iter().find(|p| p.name() == name) {
                    let graph_view = self.graph_view();
                    let graph = graph_view.graph().borrow();
                    let master_links: Vec<_> = graph.edge_references().filter(|e| {
                        let l = e.weight();
                        l.output_port().as_ref() == Some(&master) || l.input_port().as_ref() == Some(&master)
                    }).map(|e| e.weight().clone()).collect();

                    for link in master_links {
                        let other_port = if link.output_port().as_ref() == Some(&master) {
                            link.input_port().clone()
                        } else {
                            link.output_port().clone()
                        };
                        if let Some(other) = other_port {
                            let from = if direction == libspa::utils::Direction::Output { id } else { other.pw_id() };
                            let to = if direction == libspa::utils::Direction::Output { other.pw_id() } else { id };
                            self.request_link_ensure(from, to);
                        }
                    }
                }
            }
        }

        fn get_master_port_id_with_items(&self, items: &std::collections::HashMap<u32, glib::Object>, port_id: PortId) -> Option<PortId> {
            let port = items.get(&port_id.0).and_then(|i| i.dynamic_cast_ref::<graph::Port>())?;
            let node_item = items.get(&port.node_id())?;
            let node = node_item.dynamic_cast_ref::<graph::Node>()?;
            
            if !node.is_app() || node.is_detached() { return None; }
            
            let name = port.name();
            let direction = port.port_direction();
            
            node.master_ports().into_iter().find(|p| p.name() == name && p.port_direction() == direction).map(|p| p.pw_id())
        }

        fn get_master_port_id(&self, port_id: PortId) -> Option<PortId> {
            let items = self.items.borrow();
            self.get_master_port_id_with_items(&items, port_id)
        }

        fn get_node_for_port_id(&self, port_id: PortId) -> Option<graph::Node> {
            let items = self.items.borrow();
            let port = items.get(&port_id.0).and_then(|i| i.dynamic_cast_ref::<graph::Port>())?;
            items.get(&port.node_id()).and_then(|i| i.clone().dynamic_cast::<graph::Node>().ok())
        }

        fn clear(&self) {
            self.items.borrow_mut().clear();
            self.offline_nodes.borrow_mut().clear();
            self.pending_deletions.borrow_mut().clear();
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
        matrix_view: &crate::ui::MatrixView,
        connection_banner: &adw::Banner,
        sender: PwSender<GtkMessage>,
        receiver: async_channel::Receiver<PipewireMessage>,
    ) -> Self {
        let res: Self = glib::Object::builder()
            .property("graph", graph)
            .property("matrix-view", matrix_view)
            .property("connection-banner", connection_banner)
            .build();

        res.imp().attach_receiver(receiver);
        assert!(
            res.imp().pw_sender.set(sender).is_ok(),
            "Should be able to set pw_sender)"
        );

        res
    }

    pub fn save_preset(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.imp().save_preset(path)
    }

    pub fn load_preset(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.imp().load_preset(path)
    }

    pub fn force_matrix_update(&self) {
        let imp = self.imp();
        if let Some(matrix) = imp.matrix_view.borrow().as_ref() {
            if let Some(sender) = imp.pw_sender.get() {
                matrix.update_view(&imp.items.borrow(), &imp.master_links.borrow(), sender.clone());
            }
        }
    }
}

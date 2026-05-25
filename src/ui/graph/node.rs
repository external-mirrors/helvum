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

use adw::{
    glib::{self, clone},
    gtk::{self, gdk, gsk, prelude::*},
    subclass::prelude::*,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use super::{Port, PortDirection};
use crate::NodeType;

mod imp {
    use super::*;
    use std::sync::LazyLock;

    #[derive(Clone, Default)]
    pub struct SubNode {
        pub name: String,
        pub online: bool,
    }

    #[derive(glib::Properties, gtk::CompositeTemplate, Default)]
    #[properties(wrapper_type = super::Node)]
    #[template(resource = "/org/pipewire/Helvum/graph/node.ui")]
    pub struct Node {
        pub(super) node_name_text: RefCell<String>,
        pub(super) media_name_text: RefCell<String>,

        #[property(get, set)]
        pub(super) app_name: RefCell<String>,
        pub(super) node_type: Cell<Option<NodeType>>,
        pub(super) sub_nodes: RefCell<HashMap<u32, SubNode>>,

        #[property(
            name = "node-name", type = String,
            get = |this: &Self| this.node_name_text.borrow().clone(),
            set = |this: &Self, value: String| {
                this.node_name_text.replace(value.clone());
                this.node_name.set_text(&value);
            }
        )]
        #[property(
            name = "media-name", type = String,
            get = |this: &Self| this.media_name_text.borrow().clone(),
            set = |this: &Self, value: String| {
                this.media_name_text.replace(value.clone());
                this.media_name.set_text(&value);
                this.media_name.set_visible(!value.is_empty());
            }
        )]
        #[property(get, set)]
        pub(super) is_app: Cell<bool>,
        #[property(get, set)]
        pub(super) is_detached: Cell<bool>,
        #[property(get, set)]
        pub(super) is_pinned: Cell<bool>,
        #[property(get, set)]
        pub(super) auto_delete: Cell<bool>,
        #[property(get, set)]
        pub(super) online: Cell<bool>,
        #[property(get, set)]
        pub(super) pipewire_id: Cell<u32>,

        #[template_child]
        pub(super) node_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) media_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) separator: TemplateChild<gtk::Separator>,
        #[template_child]
        pub(super) port_grid: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) sub_canvas: TemplateChild<gtk::Fixed>,
        #[template_child]
        pub(super) action_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) detach_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) reattach_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) pin_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) auto_delete_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub(super) delete_button: TemplateChild<gtk::Button>,

        pub(super) ports: RefCell<HashSet<Port>>,
        pub(super) master_ports: RefCell<HashSet<Port>>,
        pub(super) sub_node_widgets: RefCell<std::collections::HashMap<u32, crate::ui::graph::SubNode>>,
        pub(super) internal_links: RefCell<std::collections::HashMap<u32, InternalLink>>,
    }

    pub struct InternalLink {
        pub output_port_id: u32,
        pub input_port_id: u32,
        pub active: bool,
        pub media_type: crate::MediaType,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Node {
        const NAME: &'static str = "HelvumNode";
        type Type = super::Node;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name("node");
            klass.bind_template();
            klass.install_action("node.delete", None, |node, _, _| {
                node.emit_by_name::<()>("delete", &[]);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Node {
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, value, pspec)
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: LazyLock<Vec<glib::subclass::Signal>> = LazyLock::new(|| {
                vec![
                    glib::subclass::Signal::builder("delete").build(),
                    glib::subclass::Signal::builder("detach").param_types([u32::static_type()]).build(),
                    glib::subclass::Signal::builder("reattach").param_types([u32::static_type()]).build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            // Bind action_box visibility to is_app
            obj.bind_property("is-app", &*self.action_box, "visible")
                .build();

            obj.bind_property("is-detached", &*self.detach_button, "visible")
                .transform_to(|_, value: bool| Some((!value).to_value()))
                .build();
            obj.bind_property("is-detached", &*self.reattach_button, "visible")
                .build();

            obj.bind_property("auto-delete", &*self.auto_delete_button, "active")
                .flags(glib::BindingFlags::BIDIRECTIONAL | glib::BindingFlags::SYNC_CREATE)
                .build();

            obj.bind_property("online", &*self.delete_button, "visible")
                .transform_to(|_, value: bool| Some((!value).to_value()))
                .build();

            self.detach_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| {
                    let sub_nodes = obj.imp().sub_nodes.borrow();
                    if let Some(&id) = sub_nodes.iter().filter(|(_, sn)| sn.online).map(|(id, _)| id).next() {
                        obj.emit_by_name::<()>("detach", &[&id]);
                    }
                }
            ));

            self.reattach_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| {
                    let sub_nodes = obj.imp().sub_nodes.borrow();
                    if let Some(&id) = sub_nodes.iter().filter(|(_, sn)| sn.online).map(|(id, _)| id).next() {
                        obj.emit_by_name::<()>("reattach", &[&id]);
                    }
                }
            ));

            self.pin_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| {
                    let is_pinned = obj.is_pinned();
                    obj.set_is_pinned(!is_pinned);
                    if !is_pinned {
                        obj.imp().pin_button.add_css_class("suggested-action");
                    } else {
                        obj.imp().pin_button.remove_css_class("suggested-action");
                    }
                }
            ));

            self.delete_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| {
                    obj.emit_by_name::<()>("delete", &[]);
                }
            ));
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Node {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.parent_snapshot(snapshot);
            
            // Draw internal links
            let links = self.internal_links.borrow();
            let widgets = self.sub_node_widgets.borrow();
            
            for link in links.values() {
                let out_port = widgets.values().find_map(|w| w.find_port(link.output_port_id))
                    .or_else(|| self.master_ports.borrow().iter().find(|p| p.pw_id().0 == link.output_port_id).cloned());
                    
                let in_port = widgets.values().find_map(|w| w.find_port(link.input_port_id))
                    .or_else(|| self.master_ports.borrow().iter().find(|p| p.pw_id().0 == link.input_port_id).cloned());

                if let (Some(out_pt), Some(in_pt)) = (out_port, in_port) {
                    if let (Some(out_p), Some(in_p)) = (
                        out_pt.compute_point(&*self.obj(), &out_pt.link_anchor()),
                        in_pt.compute_point(&*self.obj(), &in_pt.link_anchor())
                    ) {
                        self.draw_internal_link(snapshot, &out_p, &in_p, link.active, link.media_type);
                    }
                }
            }
        }
    }

    impl Node {
        fn draw_internal_link(
            &self,
            snapshot: &gtk::Snapshot,
            start: &gtk::graphene::Point,
            end: &gtk::graphene::Point,
            active: bool,
            media_type: crate::MediaType,
        ) {
            let cp1 = gtk::graphene::Point::new(start.x() + 50.0, start.y());
            let cp2 = gtk::graphene::Point::new(end.x() - 50.0, end.y());

            let builder = gsk::PathBuilder::new();
            builder.move_to(start.x(), start.y());
            builder.cubic_to(cp1.x(), cp1.y(), cp2.x(), cp2.y(), end.x(), end.y());
            let path = builder.to_path();

            let color = match media_type {
                crate::MediaType::Audio => gdk::RGBA::new(50.0 / 255.0, 100.0 / 255.0, 240.0 / 255.0, 1.0),
                crate::MediaType::Video => gdk::RGBA::new(200.0 / 255.0, 200.0 / 255.0, 0.0, 1.0),
                _ => gdk::RGBA::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0),
            };

            let stroke = gsk::Stroke::new(2.0);
            if !active {
                stroke.set_dash(&[10.0, 5.0]);
            }

            snapshot.append_stroke(&path, &stroke, &color);
        }
    }

    impl Node {
        pub fn update_ports(&self) {
            // Clear current grid
            while let Some(ref child) = self.port_grid.first_child() {
                self.port_grid.remove(child);
            }

            let ports = self.ports.borrow();
            let sub_nodes = self.sub_nodes.borrow();
            let is_app = self.is_app.get();

            if !is_app {
                // Restore old look for non-apps: single grid
                let grid = gtk::Grid::builder()
                    .column_spacing(10)
                    .margin_start(4)
                    .margin_end(4)
                    .margin_bottom(4)
                    .build();

                let mut outs: Vec<_> = ports.iter().filter(|p| p.port_direction() == PortDirection::Output).collect();
                let mut ins: Vec<_> = ports.iter().filter(|p| p.port_direction() == PortDirection::Input).collect();
                
                outs.sort_unstable_by_key(|port| port.name());
                ins.sort_unstable_by_key(|port| port.name());

                let sub_rows = std::cmp::max(outs.len(), ins.len());
                for i in 0..sub_rows {
                    if let Some(port) = ins.get(i) {
                        port.set_show_handle(true);
                        grid.attach(*port, 0, i as i32, 1, 1);
                    }
                    if let Some(port) = outs.get(i) {
                        port.set_show_handle(true);
                        grid.attach(*port, 1, i as i32, 1, 1);
                    }
                }
                self.port_grid.append(&grid);
                self.action_box.set_visible(false);
                self.separator.set_visible(!ports.is_empty());
                self.sub_canvas.set_visible(false);
                self.port_grid.queue_draw();
                return;
            }

            self.action_box.set_visible(true);

            let master_ports = self.master_ports.borrow();
            self.separator.set_visible(!master_ports.is_empty());
            
            let grid = gtk::Grid::builder()
                .column_spacing(10)
                .margin_start(4)
                .margin_end(4)
                .margin_bottom(4)
                .build();

            let mut outs: Vec<_> = master_ports.iter().filter(|p| p.port_direction() == PortDirection::Output).collect();
            let mut ins: Vec<_> = master_ports.iter().filter(|p| p.port_direction() == PortDirection::Input).collect();
            
            outs.sort_unstable_by_key(|port| port.name());
            ins.sort_unstable_by_key(|port| port.name());

            let master_rows = std::cmp::max(outs.len(), ins.len());
            for i in 0..master_rows {
                if let Some(port) = ins.get(i) {
                    port.set_show_handle(true);
                    grid.attach(*port, 0, i as i32, 1, 1);
                }
                if let Some(port) = outs.get(i) {
                    port.set_show_handle(true);
                    grid.attach(*port, 1, i as i32, 1, 1);
                }
            }
            self.port_grid.append(&grid);
            self.port_grid.queue_draw();

            if !sub_nodes.is_empty() {
                self.sub_canvas.set_visible(true);
                let mut widgets = self.sub_node_widgets.borrow_mut();
                
                widgets.retain(|id, _| sub_nodes.contains_key(id));
                
                while let Some(child) = self.sub_canvas.first_child() {
                    self.sub_canvas.remove(&child);
                }

                let mut sorted_ids: Vec<_> = sub_nodes.keys().collect();
                sorted_ids.sort();

                let existing_count = widgets.len();
                for &id in sorted_ids {
                    let sn_info = sub_nodes.get(&id).unwrap();
                    let mut is_new = false;
                    let widget = widgets.entry(id).or_insert_with(|| {
                        is_new = true;
                        let w = crate::ui::graph::SubNode::new(id, &sn_info.name);

                        let obj = self.obj().clone();
                        w.connect_local("detach", false, move |args| {
                            let sub_id = args[1].get::<u32>().unwrap();
                            obj.emit_by_name::<()>("detach", &[&sub_id]);
                            None
                        });

                        w
                    });

                    if is_new {
                        let row = existing_count / 3;
                        let col = existing_count % 3;
                        widget.set_pos(10.0 + col as f64 * 160.0, 10.0 + row as f64 * 120.0);
                    }

                    if !sn_info.online {
                        widget.add_css_class("offline");
                    } else {
                        widget.remove_css_class("offline");
                    }

                    self.sub_canvas.put(widget, widget.x(), widget.y());

                    let sub_ports: Vec<_> = ports.iter().filter(|p| p.node_id() == id).cloned().collect();
                    widget.set_ports(sub_ports);
                }

                drop(widgets);
                self.update_sub_canvas_bounds();
            } else {
                self.sub_canvas.set_visible(false);
            }
        }

        pub fn update_sub_canvas_bounds(&self) {
            let mut max_x: f64 = 200.0;
            let mut max_y: f64 = 150.0;
            
            let widgets = self.sub_node_widgets.borrow();
            for widget in widgets.values() {
                max_x = max_x.max(widget.x() + 200.0); // Rough estimate of width
                max_y = max_y.max(widget.y() + 150.0); // Rough estimate of height
            }
            
            self.sub_canvas.set_size_request(max_x as i32, max_y as i32);
        }
    }
}

glib::wrapper! {
    pub struct Node(ObjectSubclass<imp::Node>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Node {
    pub fn new(name: &str, id: crate::NodeId) -> Self {
        let node: Self = glib::Object::builder()
            .property("node-name", name)
            .property("pipewire-id", id.0)
            .build();
        node.set_online(true);
        node.add_sub_node(id.0, name);
        node
    }

    pub fn update_sub_canvas_bounds(&self) {
        self.imp().update_sub_canvas_bounds();
    }

    pub fn pw_id(&self) -> crate::NodeId {
        crate::NodeId(self.pipewire_id())
    }

    pub fn pw_name(&self) -> String {
        self.node_name()
    }

    pub fn ports(&self) -> HashSet<Port> {
        self.imp().ports.borrow().clone()
    }

    pub fn add_port(&self, port: Port) {
        {
            self.imp().ports.borrow_mut().insert(port);
        }
        self.imp().update_ports();
    }

    pub fn add_master_port(&self, port: Port) {
        {
            self.imp().master_ports.borrow_mut().insert(port);
        }
        self.imp().update_ports();
    }

    pub fn has_master_port(&self, name: &str) -> bool {
        self.imp().master_ports.borrow().iter().any(|p| p.name() == name)
    }

    pub fn master_ports(&self) -> Vec<Port> {
        self.imp().master_ports.borrow().iter().cloned().collect()
    }

    pub fn remove_port(&self, port: &Port) {
        {
            self.imp().ports.borrow_mut().remove(port);
        }
        self.imp().update_ports();
    }

    pub fn add_sub_node(&self, id: u32, name: &str) {
        {
            let mut sub_nodes = self.imp().sub_nodes.borrow_mut();
            
            let duplicate_id = sub_nodes.iter().find_map(|(&k, v)| {
                if !v.online && v.name == name { Some(k) } else { None }
            });
            
            if let Some(old_id) = duplicate_id {
                sub_nodes.remove(&old_id);
                let mut ports = self.imp().ports.borrow_mut();
                ports.retain(|p| p.node_id() != old_id);
                self.imp().sub_node_widgets.borrow_mut().remove(&old_id);
            }

            sub_nodes.insert(id, imp::SubNode { name: name.to_string(), online: true });
        }
        self.imp().update_ports();
    }

    pub fn remove_sub_node(&self, id: u32) {
        {
            let mut sub_nodes = self.imp().sub_nodes.borrow_mut();
            if self.auto_delete() {
                sub_nodes.remove(&id);
            } else {
                if let Some(sn) = sub_nodes.get_mut(&id) {
                    sn.online = false;
                }
            }
        }
        self.imp().update_ports();
    }

    pub fn delete_sub_node(&self, id: u32) {
        {
            self.imp().sub_nodes.borrow_mut().remove(&id);
        }
        self.imp().update_ports();
    }

    pub fn has_sub_node(&self, id: u32) -> bool {
        self.imp().sub_nodes.borrow().get(&id).map(|sn| sn.online).unwrap_or(false)
    }

    pub fn node_type(&self) -> Option<NodeType> {
        self.imp().node_type.get()
    }

    pub fn set_node_type(&self, node_type: Option<NodeType>) {
        self.imp().node_type.set(node_type);
    }

    pub fn sub_node_count(&self) -> usize {
        self.imp().sub_nodes.borrow().values().filter(|sn| sn.online).count()
    }

    pub fn sub_nodes(&self) -> HashMap<u32, String> {
        self.imp().sub_nodes.borrow().iter().map(|(k, v)| (*k, v.name.clone())).collect()
    }


    pub fn internal_name(&self) -> String {
        self.media_name()
    }

    pub fn add_internal_link(&self, id: u32, out_id: u32, in_id: u32, active: bool, media_type: crate::MediaType) {
        self.imp().internal_links.borrow_mut().insert(id, imp::InternalLink {
            output_port_id: out_id,
            input_port_id: in_id,
            active,
            media_type,
        });
        self.queue_draw();
    }

    pub fn remove_internal_link(&self, id: u32) {
        self.imp().internal_links.borrow_mut().remove(&id);
        self.queue_draw();
    }

    pub fn update_internal_link_state(&self, id: u32, active: bool) {
        if let Some(link) = self.imp().internal_links.borrow_mut().get_mut(&id) {
            link.active = active;
            self.queue_draw();
        }
    }

    pub fn update_internal_link_format(&self, id: u32, media_type: libspa::param::format::MediaType) {
        if let Some(link) = self.imp().internal_links.borrow_mut().get_mut(&id) {
            link.media_type = media_type.into();
            self.queue_draw();
        }
    }
}

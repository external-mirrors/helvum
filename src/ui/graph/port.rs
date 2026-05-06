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
    gdk,
    glib::{self, subclass::Signal},
    gtk::{self, graphene},
    prelude::*,
    subclass::prelude::*,
};
pub use imp::{PortDirection, PortMediaType};
use crate::PortId;
use libspa::{param::format::MediaType, utils::Direction};

use super::PortHandle;

mod imp {
    use super::*;

    use std::cell::Cell;

    use libspa::param::format::MediaType;
    use std::sync::LazyLock;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, glib::Enum)]
    #[enum_type(name = "HelvumPortDirection")]
    pub enum PortDirection {
        Input,
        #[default]
        Output,
    }

    impl From<Direction> for PortDirection {
        fn from(direction: Direction) -> Self {
            match direction {
                Direction::Input => PortDirection::Input,
                Direction::Output => PortDirection::Output,
                _ => PortDirection::Input,
            }
        }
    }

    impl From<PortDirection> for Direction {
        fn from(direction: PortDirection) -> Self {
            match direction {
                PortDirection::Input => Direction::Input,
                PortDirection::Output => Direction::Output,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, glib::Enum)]
    #[enum_type(name = "HelvumPortMediaType")]
    pub enum PortMediaType {
        #[default]
        Unknown,
        Audio,
        Video,
        Application,
        Stream,
    }

    impl From<MediaType> for PortMediaType {
        fn from(m: MediaType) -> Self {
            match m {
                MediaType::Audio => PortMediaType::Audio,
                MediaType::Video => PortMediaType::Video,
                MediaType::Application => PortMediaType::Application,
                MediaType::Stream => PortMediaType::Stream,
                _ => PortMediaType::Unknown,
            }
        }
    }

    impl From<PortMediaType> for MediaType {
        fn from(m: PortMediaType) -> Self {
            match m {
                PortMediaType::Audio => MediaType::Audio,
                PortMediaType::Video => MediaType::Video,
                PortMediaType::Application => MediaType::Application,
                PortMediaType::Stream => MediaType::Stream,
                _ => MediaType::Unknown,
            }
        }
    }

    /// Graphical representation of a pipewire port.
    #[derive(gtk::CompositeTemplate)]
    #[template(file = "port.ui")]
    pub struct Port {
        pub(super) pipewire_id: Cell<u32>,
        pub(super) media_type: Cell<PortMediaType>,
        pub(super) direction: Cell<PortDirection>,
        #[template_child]
        pub(super) label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) handle: TemplateChild<PortHandle>,
    }

    impl Default for Port {
        fn default() -> Self {
            Self {
                pipewire_id: Cell::new(0),
                media_type: Cell::new(PortMediaType::Unknown),
                direction: Cell::new(PortDirection::Output),
                label: TemplateChild::default(),
                handle: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Port {
        const NAME: &'static str = "HelvumPort";
        type Type = super::Port;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("port");

            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Port {
        fn constructed(&self) {
            self.parent_constructed();

            // Force left-to-right direction for the ports grid to avoid messed up UI when defaulting to right-to-left
            gtk::prelude::WidgetExt::set_direction(&*self.obj(), gtk::TextDirection::Ltr);

            // Initial UI update
            self.update_ui_for_direction();

            // Display a grab cursor when the mouse is over the port so the user knows it can be dragged to another port.
            self.obj()
                .set_cursor(gtk::gdk::Cursor::from_name("grab", None).as_ref());

            self.setup_port_drag_and_drop();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![Signal::builder("port-toggled")
                    // Provide id of output port and input port to signal handler.
                    .param_types([<u32>::static_type(), <u32>::static_type()])
                    .build()]
            });

            SIGNALS.as_ref()
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
                vec![
                    glib::ParamSpecEnum::builder::<PortDirection>("port-direction")
                        .construct_only()
                        .build(),
                    glib::ParamSpecUInt::builder("pipewire-id")
                        .construct_only()
                        .build(),
                    glib::ParamSpecEnum::builder::<PortMediaType>("media-type")
                        .build(),
                    glib::ParamSpecString::builder("name")
                        .build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "port-direction" => {
                    let val = value.get().expect("Value should be a PortDirection");
                    self.direction.set(val);
                    self.update_ui_for_direction();
                }
                "pipewire-id" => {
                    self.pipewire_id.set(value.get().expect("Value should be a u32"));
                }
                "media-type" => {
                    let val = value.get().expect("Value should be a PortMediaType");
                    self.set_media_type(val);
                }
                "name" => {
                    let val: String = value.get().expect("Value should be a String");
                    self.label.set_text(&val);
                    self.label.set_tooltip_text(Some(&val));
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "port-direction" => self.direction.get().to_value(),
                "pipewire-id" => self.pipewire_id.get().to_value(),
                "media-type" => self.media_type.get().to_value(),
                "name" => self.label.text().to_string().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for Port {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => {
                    let (min_handle_width, nat_handle_width, _, _) =
                        self.handle.measure(orientation, for_size);
                    let (min_label_width, nat_label_width, _, _) = self
                        .label
                        .measure(orientation, i32::max(for_size - (nat_handle_width / 2), -1));

                    (
                        (min_handle_width / 2) + min_label_width,
                        (nat_handle_width / 2) + nat_label_width,
                        -1,
                        -1,
                    )
                }
                gtk::Orientation::Vertical => {
                    let (min_label_height, nat_label_height, _, _) =
                        self.label.measure(orientation, for_size);
                    let (min_handle_height, nat_handle_height, _, _) =
                        self.handle.measure(orientation, for_size);

                    (
                        i32::max(min_label_height, min_handle_height),
                        i32::max(nat_label_height, nat_handle_height),
                        -1,
                        -1,
                    )
                }
                _ => unimplemented!(),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let (_, nat_handle_height, _, _) =
                self.handle.measure(gtk::Orientation::Vertical, height);
            let (_, nat_handle_width, _, _) =
                self.handle.measure(gtk::Orientation::Horizontal, width);

            match self.obj().port_direction() {
                PortDirection::Input => {
                    let alloc = gtk::Allocation::new(
                        -nat_handle_width / 2,
                        (height - nat_handle_height) / 2,
                        nat_handle_width,
                        nat_handle_height,
                    );
                    self.handle.size_allocate(&alloc, -1);

                    let alloc = gtk::Allocation::new(
                        nat_handle_width / 2,
                        0,
                        width - (nat_handle_width / 2),
                        height,
                    );
                    self.label.size_allocate(&alloc, -1);
                }
                PortDirection::Output => {
                    let alloc = gtk::Allocation::new(
                        width - (nat_handle_width / 2),
                        (height - nat_handle_height) / 2,
                        nat_handle_width,
                        nat_handle_height,
                    );
                    self.handle.size_allocate(&alloc, -1);

                    let alloc = gtk::Allocation::new(0, 0, width - (nat_handle_width / 2), height);
                    self.label.size_allocate(&alloc, -1);
                }
            }
        }
    }

    impl Port {
        fn setup_port_drag_and_drop(&self) {
            let obj = &*self.obj();

            // Add a drag source and drop target controller with the type depending on direction,
            // they will be responsible for link creation by dragging an output port onto an input port or the other way around.
            // The port will simply provide its pipewire id to the drag target.
            // The drop target will accept the source port and use it to emit its `port-toggled` signal.

            // FIXME: We should protect against different media types, e.g. it should not be possible to drop a video port on an audio port.

            let drag_src = gtk::DragSource::builder()
                .content(&gdk::ContentProvider::for_value(&obj.to_value()))
                .build();
            // Override the default drag icon with an empty one so that only a grab cursor is shown.
            // The graph will render a link from the source port to the cursor to visualize the drag instead.
            drag_src.set_icon(Some(&gdk::Paintable::new_empty(0, 0)), 0, 0);
            drag_src.connect_drag_begin(|drag_source, _| {
                let port = drag_source
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::Port>()
                    .expect("Widget should be a Port");

                log::trace!("Drag started from port {}", port.pw_id());
            });
            drag_src.connect_drag_cancel(|drag_source, _, _| {
                let port = drag_source
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::Port>()
                    .expect("Widget should be a Port");

                log::trace!("Drag from port {} was cancelled", port.pw_id());

                false
            });
            obj.add_controller(drag_src);

            let drop_target =
                gtk::DropTarget::new(super::Port::static_type(), gdk::DragAction::COPY);
            drop_target.set_preload(true);
            drop_target.connect_value_notify(|drop_target| {
                let port = drop_target
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::Port>()
                    .expect("Widget should be a Port");

                let Some(value) = drop_target.value() else {
                    return;
                };

                let other_port: super::Port = value.get().expect("Drop value should be a port");

                // Disallow drags between two ports that have the same direction
                if !port.is_linkable_to(&other_port) {
                    // FIXME: For some reason, this prints error:
                    //        "gdk_drop_get_actions: assertion 'GDK_IS_DROP (self)' failed"
                    drop_target.reject();
                }
            });
            drop_target.connect_drop(|drop_target, val, _, _| {
                let port = drop_target
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::Port>()
                    .expect("Widget should be a Port");
                let other_port = val
                    .get::<super::Port>()
                    .expect("Dropped value should be a Port");

                // Do not accept a drop between imcompatible ports
                if !port.is_linkable_to(&other_port) {
                    log::warn!("Tried to link incompatible ports");
                    return false;
                }

                let (output_port, input_port) = match port.port_direction() {
                    PortDirection::Output => (&port, &other_port),
                    PortDirection::Input => (&other_port, &port),
                };

                port.emit_by_name::<()>(
                    "port-toggled",
                    &[&output_port.pw_id().0, &input_port.pw_id().0],
                );

                true
            });
            obj.add_controller(drop_target);
        }

        fn set_media_type(&self, media_type: PortMediaType) {
            self.media_type.set(media_type);

            for css_class in ["video", "audio", "midi"] {
                self.handle.remove_css_class(css_class)
            }

            // Color the port according to its media type.
            match MediaType::from(media_type) {
                MediaType::Video => self.handle.add_css_class("video"),
                MediaType::Audio => self.handle.add_css_class("audio"),
                MediaType::Application | MediaType::Stream => self.handle.add_css_class("midi"),
                _ => {}
            }
        }

        fn update_ui_for_direction(&self) {
            let direction = self.direction.get();

            match direction {
                PortDirection::Input => {
                    self.obj().set_halign(gtk::Align::Start);
                    self.label.set_halign(gtk::Align::Start);
                }
                PortDirection::Output => {
                    self.obj().set_halign(gtk::Align::End);
                    self.label.set_halign(gtk::Align::End);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct Port(ObjectSubclass<imp::Port>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Port {
    pub fn new(id: PortId, name: &str, direction: Direction) -> Self {
        glib::Object::builder()
            .property("pipewire-id", id.0)
            .property("port-direction", PortDirection::from(direction))
            .property("name", name)
            .build()
    }

    pub fn port_direction(&self) -> PortDirection {
        self.property("port-direction")
    }

    pub fn name(&self) -> String {
        self.property("name")
    }

    pub fn set_media_type(&self, media_type: MediaType) {
        self.set_property("media-type", PortMediaType::from(media_type));
    }

    pub fn media_type(&self) -> PortMediaType {
        self.property("media-type")
    }

    pub fn pw_id(&self) -> PortId {
        PortId(self.property("pipewire-id"))
    }

    pub fn link_anchor(&self) -> graphene::Point {
        let imp = self.imp();
        let handle = &imp.handle;
        let (width, height) = (handle.width() as f32, handle.height() as f32);

        handle
            .compute_point(self, &graphene::Point::new(width / 2.0, height / 2.0))
            .expect("Failed to compute link anchor")
    }

    pub fn is_linkable_to(&self, other_port: &Self) -> bool {
        self.port_direction() != other_port.port_direction()
    }
}

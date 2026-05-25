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

use adw::{glib, gtk, prelude::*, subclass::prelude::*};
use libspa::param::format::MediaType;

use super::{Port, PortMediaType};

mod imp {
    use super::*;

    use std::cell::Cell;

    use std::sync::LazyLock;

    pub struct Link {
        pub output_port: glib::WeakRef<Port>,
        pub input_port: glib::WeakRef<Port>,
        pub active: Cell<bool>,
        pub online: Cell<bool>,
        pub media_type: Cell<PortMediaType>,
        pub pending_check: Cell<bool>,
    }

    impl Default for Link {
        fn default() -> Self {
            Self {
                output_port: glib::WeakRef::default(),
                input_port: glib::WeakRef::default(),
                active: Cell::default(),
                online: Cell::new(true),
                media_type: Cell::new(PortMediaType::Unknown),
                pending_check: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Link {
        const NAME: &'static str = "HelvumLink";
        type Type = super::Link;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Link {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<Port>("output-port")
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                    glib::ParamSpecObject::builder::<Port>("input-port")
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                    glib::ParamSpecBoolean::builder("active")
                        .default_value(false)
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                    glib::ParamSpecBoolean::builder("online")
                        .default_value(true)
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                    glib::ParamSpecBoolean::builder("pending-check")
                        .default_value(false)
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                    glib::ParamSpecEnum::builder::<PortMediaType>("media-type")
                        .flags(glib::ParamFlags::READWRITE)
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "output-port" => self.output_port.upgrade().to_value(),
                "input-port" => self.input_port.upgrade().to_value(),
                "active" => self.active.get().to_value(),
                "online" => self.online.get().to_value(),
                "pending-check" => self.pending_check.get().to_value(),
                "media-type" => self.media_type.get().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.update_property(&[gtk::accessible::Property::Label("Link")]);
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "output-port" => {
                    self.output_port.set(value.get().unwrap());
                    self.update_accessible_label();
                }
                "input-port" => {
                    self.input_port.set(value.get().unwrap());
                    self.update_accessible_label();
                }
                "active" => {
                    self.active.set(value.get().unwrap());
                    self.update_accessible_label();
                }
                "online" => self.online.set(value.get().unwrap()),
                "pending-check" => self.pending_check.set(value.get().unwrap()),
                "media-type" => self
                    .media_type
                    .set(value.get().expect("Value should be a PortMediaType")),
                _ => unimplemented!(),
            }
        }
    }
    
    impl Link {
        fn update_accessible_label(&self) {
            let out_name = self.output_port.upgrade().map(|p| p.name()).unwrap_or_default();
            let in_name = self.input_port.upgrade().map(|p| p.name()).unwrap_or_default();
            let status = if self.active.get() { "Active" } else { "Inactive" };
            let label = format!("{} link from {} to {}", status, out_name, in_name);
            self.obj().update_property(&[gtk::accessible::Property::Label(&label)]);
        }
    }

    impl WidgetImpl for Link {}
}

glib::wrapper! {
    pub struct Link(ObjectSubclass<imp::Link>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Link {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn output_port(&self) -> Option<Port> {
        self.property("output-port")
    }

    pub fn set_output_port(&self, port: Option<&Port>) {
        self.set_property("output-port", port);
    }

    pub fn input_port(&self) -> Option<Port> {
        self.property("input-port")
    }

    pub fn set_input_port(&self, port: Option<&Port>) {
        self.set_property("input-port", port);
    }

    pub fn active(&self) -> bool {
        self.property("active")
    }

    pub fn set_active(&self, active: bool) {
        self.set_property("active", active);
    }

    pub fn online(&self) -> bool {
        self.property("online")
    }

    pub fn set_online(&self, online: bool) {
        self.set_property("online", online);
    }

    pub fn media_type(&self) -> PortMediaType {
        self.property("media-type")
    }

    pub fn set_media_type(&self, media_type: MediaType) {
        self.set_property("media-type", PortMediaType::from(media_type))
    }

    pub fn pending_check(&self) -> bool {
        self.property("pending-check")
    }

    pub fn set_pending_check(&self, pending: bool) {
        self.set_property("pending-check", pending);
    }
}

impl Default for Link {
    fn default() -> Self {
        Self::new()
    }
}

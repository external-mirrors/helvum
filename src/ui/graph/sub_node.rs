use adw::{
    glib::{self, clone, subclass::Signal},
    gtk::{self, prelude::*},
    subclass::prelude::*,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::LazyLock;
use crate::ui::graph::Port;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/org/pipewire/Helvum/graph/sub_node.ui")]
    pub struct SubNode {
        #[template_child]
        pub(super) name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) port_grid: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) detach_button: TemplateChild<gtk::Button>,

        pub(super) ports: RefCell<HashSet<Port>>,
        pub(super) id: Cell<u32>,
        pub x: Cell<f64>,
        pub y: Cell<f64>,
        pub(super) drag_start_pos: Cell<(f64, f64)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SubNode {
        const NAME: &'static str = "HelvumSubNode";
        type Type = super::SubNode;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SubNode {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![Signal::builder("detach")
                    .param_types([u32::static_type()])
                    .build()]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            
            let obj = self.obj();
            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin(clone!(
                #[weak]
                obj,
                move |gesture, x, y| {
                    if let Some(_parent) = obj.parent().and_downcast::<gtk::Fixed>() {
                        obj.imp().drag_start_pos.set((x, y));
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            drag.connect_drag_update(clone!(
                #[weak]
                obj,
                move |_, offset_x, offset_y| {
                    if let Some(parent) = obj.parent().and_downcast::<gtk::Fixed>() {
                        let mut new_x = obj.imp().x.get() + offset_x;
                        let mut new_y = obj.imp().y.get() + offset_y;
                        
                        if new_x < 0.0 { new_x = 0.0; }
                        if new_y < 0.0 { new_y = 0.0; }
                        
                        parent.move_(&obj, new_x, new_y);
                        
                        if let Some(node) = parent.parent().and_then(|p| p.parent()).and_downcast::<crate::ui::graph::Node>() {
                            node.update_sub_canvas_bounds();
                            node.queue_draw();
                        }
                    }
                }
            ));
            drag.connect_drag_end(clone!(
                #[weak]
                obj,
                move |_, offset_x, offset_y| {
                    let mut new_x = obj.imp().x.get() + offset_x;
                    let mut new_y = obj.imp().y.get() + offset_y;
                    if new_x < 0.0 { new_x = 0.0; }
                    if new_y < 0.0 { new_y = 0.0; }
                    obj.imp().x.set(new_x);
                    obj.imp().y.set(new_y);
                }
            ));
            obj.add_controller(drag);
        }
    }
    
    impl WidgetImpl for SubNode {}
}

glib::wrapper! {
    pub struct SubNode(ObjectSubclass<imp::SubNode>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SubNode {
    pub fn new(id: u32, name: &str) -> Self {
        let obj: Self = glib::Object::builder()
            .build();
        obj.imp().id.set(id);
        obj.imp().name_label.set_label(name);

        let self_clone = obj.clone();
        obj.imp().detach_button.connect_clicked(move |_| {
            self_clone.emit_by_name::<()>("detach", &[&id]);
        });

        obj
    }

    pub fn id(&self) -> u32 {
        self.imp().id.get()
    }

    pub fn x(&self) -> f64 {
        self.imp().x.get()
    }

    pub fn y(&self) -> f64 {
        self.imp().y.get()
    }

    pub fn set_pos(&self, x: f64, y: f64) {
        self.imp().x.set(x);
        self.imp().y.set(y);
    }

    pub fn add_port(&self, port: Port) {
        self.imp().ports.borrow_mut().insert(port.clone());
        self.update_ports();
    }

    pub fn remove_port(&self, port: &Port) {
        self.imp().ports.borrow_mut().remove(port);
        self.update_ports();
    }

    fn update_ports(&self) {
        let imp = self.imp();
        while let Some(child) = imp.port_grid.first_child() {
            imp.port_grid.remove(&child);
        }

        let ports = imp.ports.borrow();
        let grid = gtk::Grid::builder()
            .column_spacing(6)
            .build();

        let mut outs: Vec<_> = ports.iter().filter(|p| p.port_direction() == crate::ui::graph::PortDirection::Output).collect();
        let mut ins: Vec<_> = ports.iter().filter(|p| p.port_direction() == crate::ui::graph::PortDirection::Input).collect();
        
        outs.sort_unstable_by_key(|port| port.name());
        ins.sort_unstable_by_key(|port| port.name());

        let rows = std::cmp::max(outs.len(), ins.len());
        for i in 0..rows {
            if let Some(port) = ins.get(i) {
                port.set_show_handle(true);
                grid.attach(*port, 0, i as i32, 1, 1);
            }
            if let Some(port) = outs.get(i) {
                port.set_show_handle(true);
                grid.attach(*port, 1, i as i32, 1, 1);
            }
        }
        imp.port_grid.append(&grid);
    }

    pub fn set_ports(&self, ports: Vec<Port>) {
        {
            let mut self_ports = self.imp().ports.borrow_mut();
            self_ports.clear();
            for p in ports {
                self_ports.insert(p);
            }
        }
        self.update_ports();
    }

    pub fn find_port(&self, pw_id: u32) -> Option<Port> {
        self.imp().ports.borrow().iter().find(|p| p.pw_id() == crate::PortId(pw_id)).cloned()
    }
}

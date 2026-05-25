use adw::{glib, gtk, prelude::*, subclass::prelude::*};

#[derive(Clone)]
pub struct PortInfo {
    pub pw_id: crate::PortId,
    pub node_name: String,
    pub port_name: String,
}

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct MatrixView {
        pub out_list: gtk::ListBox,
        pub in_list: gtk::ListBox,
        pub paned: gtk::Paned,
        pub selected_out: std::cell::Cell<Option<crate::PortId>>,
        pub pw_sender: RefCell<Option<pipewire::channel::Sender<crate::GtkMessage>>>,
        pub links: RefCell<std::collections::HashSet<(crate::PortId, crate::PortId)>>,
        pub in_ports_data: RefCell<Vec<super::PortInfo>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MatrixView {
        const NAME: &'static str = "HelvumMatrixView";
        type Type = super::MatrixView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for MatrixView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_layout_manager(Some(gtk::BinLayout::new()));
            
            self.paned.set_orientation(gtk::Orientation::Horizontal);
            self.paned.set_position(300); // initial split
            self.paned.set_wide_handle(true);
            
            let left_scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .child(&self.out_list)
                .build();
            
            let right_scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .child(&self.in_list)
                .build();
                
            self.paned.set_start_child(Some(&left_scroll));
            self.paned.set_end_child(Some(&right_scroll));
            
            self.out_list.connect_row_selected(glib::clone!(
                #[weak(rename_to = imp)] self,
                move |_, row| {
                    if let Some(row) = row {
                        if let Some(name) = row.widget_name().as_str().parse::<u32>().ok() {
                            imp.selected_out.set(Some(crate::PortId(name)));
                            imp.obj().refresh_in_list();
                        }
                    } else {
                        imp.selected_out.set(None);
                        imp.obj().refresh_in_list();
                    }
                }
            ));
            
            self.paned.set_parent(&*obj);
        }
        
        fn dispose(&self) {
            if let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for MatrixView {}
}

glib::wrapper! {
    pub struct MatrixView(ObjectSubclass<imp::MatrixView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MatrixView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn update_view(
        &self, 
        items: &std::collections::HashMap<u32, glib::Object>, 
        master_links: &std::collections::HashMap<String, crate::ui::graph::Link>,
        pw_sender: pipewire::channel::Sender<crate::GtkMessage>
    ) {
        let imp = self.imp();
        *imp.pw_sender.borrow_mut() = Some(pw_sender);
        
        let mut out_ports = Vec::new();
        let mut in_ports = Vec::new();
        let mut links: std::collections::HashSet<(crate::PortId, crate::PortId)> = std::collections::HashSet::new();

        for item in items.values() {
            if let Ok(port) = item.clone().downcast::<crate::ui::graph::Port>() {
                let node_id = port.node_id();
                let node_name = if let Some(n) = items.get(&node_id) {
                    if let Ok(node) = n.clone().downcast::<crate::ui::graph::Node>() {
                        node.pw_name()
                    } else { String::new() }
                } else { String::new() };
                
                let info = PortInfo {
                    pw_id: port.pw_id(),
                    node_name,
                    port_name: port.name(),
                };
                
                match port.port_direction() {
                    crate::ui::graph::PortDirection::Output => out_ports.push(info),
                    crate::ui::graph::PortDirection::Input => in_ports.push(info),
                }
            }
        }
        
        for link in master_links.values() {
            if let (Some(out_port), Some(in_port)) = (link.output_port(), link.input_port()) {
                links.insert((out_port.pw_id(), in_port.pw_id()));
            }
        }
        
        out_ports.sort_by(|a, b| a.node_name.cmp(&b.node_name).then(a.port_name.cmp(&b.port_name)));
        in_ports.sort_by(|a, b| a.node_name.cmp(&b.node_name).then(a.port_name.cmp(&b.port_name)));
        
        *imp.links.borrow_mut() = links;
        *imp.in_ports_data.borrow_mut() = in_ports;
        
        while let Some(child) = imp.out_list.first_child() {
            imp.out_list.remove(&child);
        }
        
        let selected_id = imp.selected_out.get();
        let mut row_to_select = None;
        
        for out_port in out_ports {
            let label = gtk::Label::builder()
                .label(&format!("{}: {}", out_port.node_name, out_port.port_name))
                .halign(gtk::Align::Start)
                .margin_start(6)
                .margin_end(6)
                .margin_top(6)
                .margin_bottom(6)
                .build();
                
            let row = gtk::ListBoxRow::builder()
                .child(&label)
                .name(&out_port.pw_id.0.to_string())
                .build();
                
            imp.out_list.append(&row);
            
            if Some(out_port.pw_id) == selected_id {
                row_to_select = Some(row);
            }
        }
        
        if let Some(row) = row_to_select {
            imp.out_list.select_row(Some(&row));
        } else {
            imp.selected_out.set(None);
            self.refresh_in_list();
        }
    }

    fn refresh_in_list(&self) {
        let imp = self.imp();
        while let Some(child) = imp.in_list.first_child() {
            imp.in_list.remove(&child);
        }
        
        let selected_out = imp.selected_out.get();
        if selected_out.is_none() {
            let placeholder = gtk::Label::builder()
                .label("Select an output port on the left")
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .margin_top(24)
                .build();
            let row = gtk::ListBoxRow::builder().child(&placeholder).activatable(false).selectable(false).build();
            imp.in_list.append(&row);
            return;
        }
        
        let out_id = selected_out.unwrap();
        let links = imp.links.borrow();
        let pw_sender = imp.pw_sender.borrow().clone();
        
        for in_port in imp.in_ports_data.borrow().iter() {
            let is_linked = links.contains(&(out_id, in_port.pw_id));
            
            let box_ = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .margin_start(6)
                .margin_end(6)
                .margin_top(6)
                .margin_bottom(6)
                .build();
                
            let cb = gtk::CheckButton::builder()
                .active(is_linked)
                .build();
                
            let label = gtk::Label::builder()
                .label(&format!("{}: {}", in_port.node_name, in_port.port_name))
                .halign(gtk::Align::Start)
                .hexpand(true)
                .build();
                
            box_.append(&cb);
            box_.append(&label);
            
            let row = gtk::ListBoxRow::builder()
                .child(&box_)
                .name(&in_port.pw_id.0.to_string())
                .activatable(true)
                .build();
                
            let in_id = in_port.pw_id;
            
            if let Some(sender) = pw_sender.as_ref() {
                let sender = sender.clone();
                cb.connect_toggled(move |_btn| {
                    let _ = sender.send(crate::GtkMessage::ToggleLink { 
                        port_from: out_id, 
                        port_to: in_id 
                    });
                });
                
                let cb_clone = cb.clone();
                row.connect_activate(move |_| {
                    cb_clone.set_active(!cb_clone.is_active());
                });
            }
            
            imp.in_list.append(&row);
        }
    }
}

impl Default for MatrixView {
    fn default() -> Self {
        Self::new()
    }
}

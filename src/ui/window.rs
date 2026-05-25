use adw::{gio, glib, glib::clone, gtk, prelude::*, subclass::prelude::*};

use super::graph;

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::Window)]
    #[template(resource = "/org/pipewire/Helvum/window.ui")]
    pub struct Window {
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        #[property(type = adw::Banner, get = |_| self.connection_banner.clone())]
        pub connection_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,

        #[property(type = graph::GraphView, get = |this: &Self| this.graph.clone())]
        pub graph: graph::GraphView,
        #[property(type = crate::ui::MatrixView, get = |this: &Self| this.matrix_view.clone())]
        pub matrix_view: crate::ui::MatrixView,
        pub is_matrix_view: std::cell::Cell<bool>,
        pub zoom_entry: graph::ZoomEntry,
    }

    impl Default for Window {
        fn default() -> Self {
            let graph = graph::GraphView::new();
            let matrix_view = crate::ui::MatrixView::new();
            // We'll set the zoomed widget later in constructed
            let zoom_entry = glib::Object::new::<graph::ZoomEntry>();

            Self {
                header_bar: TemplateChild::default(),
                connection_banner: TemplateChild::default(),
                overlay: TemplateChild::default(),
                scrolled_window: TemplateChild::default(),
                graph,
                matrix_view,
                is_matrix_view: std::cell::Cell::new(false),
                zoom_entry,
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "HelvumWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            // Ensure custom types are registered
            graph::GraphView::ensure_type();
            crate::ui::MatrixView::ensure_type();
            graph::ZoomEntry::ensure_type();

            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();

            self.scrolled_window.set_child(Some(&self.graph));

            self.zoom_entry.set_halign(gtk::Align::End);
            self.zoom_entry.set_valign(gtk::Align::End);
            self.zoom_entry.set_margin_end(24);
            self.zoom_entry.set_margin_bottom(24);
            self.zoom_entry.set_property("zoomed-widget", &self.graph);

            self.overlay.add_overlay(&self.zoom_entry);
        }
    }
    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager, gio::ActionGroup, gio::ActionMap;
}

use crate::application::Application;

impl Window {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn setup_actions(&self) {
        let save_action = gio::SimpleAction::new("save-preset", None);
        save_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Save Preset");
                dialog.set_initial_name(Some("preset.toml"));

                dialog.save(Some(&window), gio::Cancellable::NONE, clone!(#[weak] window, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            let path_str = path.to_string_lossy();
                            let app = window.application().unwrap().dynamic_cast::<Application>().unwrap();
                            let gm = app.imp().graph_manager.get().unwrap();
                            if let Err(e) = gm.save_preset(&path_str) {
                                log::error!("Failed to save preset: {}", e);
                            }
                        }
                    }
                }));
            }
        ));
        self.add_action(&save_action);

        let load_action = gio::SimpleAction::new("load-preset", None);
        load_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Load Preset");

                dialog.open(Some(&window), gio::Cancellable::NONE, clone!(#[weak] window, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            let path_str = path.to_string_lossy();
                            let app = window.application().unwrap().dynamic_cast::<Application>().unwrap();
                            let gm = app.imp().graph_manager.get().unwrap();
                            if let Err(e) = gm.load_preset(&path_str) {
                                log::error!("Failed to load preset: {}", e);
                            }
                        }
                    }
                }));
            }
        ));
        self.add_action(&load_action);

        let toggle_view_action = gio::SimpleAction::new("toggle-view", None);
        toggle_view_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let imp = window.imp();
                let is_matrix = imp.is_matrix_view.get();
                if is_matrix {
                    imp.scrolled_window.set_child(Some(&imp.graph));
                    imp.is_matrix_view.set(false);
                } else {
                    imp.scrolled_window.set_child(Some(&imp.matrix_view));
                    imp.is_matrix_view.set(true);
                    
                    if let Some(app) = window.application() {
                        if let Ok(app) = app.downcast::<Application>() {
                            if let Some(gm) = app.imp().graph_manager.get() {
                                gm.force_matrix_update();
                            }
                        }
                    }
                }
            }
        ));
        self.add_action(&toggle_view_action);
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

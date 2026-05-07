use adw::{gio, gtk, prelude::*, subclass::prelude::*};

use super::graph;

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::Window)]
    #[template(file = "window.ui")]
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
        pub zoom_entry: graph::ZoomEntry,
    }

    impl Default for Window {
        fn default() -> Self {
            let graph = graph::GraphView::new();
            // We'll set the zoomed widget later in constructed
            let zoom_entry = glib::Object::new::<graph::ZoomEntry>();

            Self {
                header_bar: TemplateChild::default(),
                connection_banner: TemplateChild::default(),
                overlay: TemplateChild::default(),
                scrolled_window: TemplateChild::default(),
                graph,
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

impl Window {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

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
    gio,
    glib::{self, clone},
    gtk::{
        self,
        graphene::{self, Point},
        gsk,
    },
    prelude::*,
    subclass::prelude::*,
};

use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Bfs, EdgeRef, IntoEdgeReferences, IntoNodeReferences, Reversed};
use petgraph::Directed;
use std::cell::RefCell;

use super::{Link, Node, Port};
use crate::NodeType;

const CANVAS_SIZE: f64 = 5000.0;

pub struct NodeWeight {
    pub widget: Node,
    pub position: Point,
}

mod imp {
    use super::*;

    use petgraph::stable_graph::{EdgeIndex, NodeIndex};
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet};

    use crate::ui::graph::PortDirection;
    use adw::gtk::gdk;
    use libspa::param::format::MediaType;
    use log::warn;
    use std::sync::LazyLock;

    pub struct Colors {
        audio: gdk::RGBA,
        video: gdk::RGBA,
        midi: gdk::RGBA,
        unknown: gdk::RGBA,
    }

    impl Colors {
        pub fn color_for_media_type(&self, media_type: MediaType) -> &gdk::RGBA {
            match media_type {
                MediaType::Audio => &self.audio,
                MediaType::Video => &self.video,
                MediaType::Stream | MediaType::Application => &self.midi,
                _ => &self.unknown,
            }
        }
    }

    pub struct DragState {
        node: glib::WeakRef<Node>,
        /// This stores the offset of the pointer to the origin of the node,
        /// so that we can keep the pointer over the same position when moving the node
        ///
        /// The offset is normalized to the default zoom-level of 1.0.
        offset: Point,
    }

    pub struct GraphView {
        /// Stores the topological graph.
        pub(crate) graph: RefCell<StableGraph<NodeWeight, Link, Directed>>,
        /// Fast lookup for nodes.
        pub(super) node_to_index: RefCell<HashMap<Node, NodeIndex>>,
        /// Fast lookup for links.
        pub(super) link_to_index: RefCell<HashMap<Link, EdgeIndex>>,

        // Properties for zooming and scrolling the hraph
        pub hadjustment: RefCell<Option<gtk::Adjustment>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub zoom_factor: Cell<f64>,

        /// This keeps track of an ongoing node drag operation.
        pub dragged_node: RefCell<Option<DragState>>,

        // These keep track of an ongoing port drag operation
        pub dragged_port: glib::WeakRef<Port>,
        pub port_drag_cursor: Cell<Point>,

        // Memorized data for an in-progress zoom gesture
        pub zoom_gesture_initial_zoom: Cell<Option<f64>>,
        pub zoom_gesture_anchor: Cell<Option<(f64, f64)>>,

        // This keeps track of an ongoing move view gesture.
        pub move_view_state: Cell<(f64, f64)>,
        pub highlighted_nodes: RefCell<HashSet<NodeIndex>>,
        pub highlighted_edges: RefCell<HashSet<EdgeIndex>>,
    }

    impl Default for GraphView {
        fn default() -> Self {
            Self {
                graph: Default::default(),
                node_to_index: Default::default(),
                link_to_index: Default::default(),
                hadjustment: Default::default(),
                vadjustment: Default::default(),
                zoom_factor: Default::default(),
                dragged_node: Default::default(),
                dragged_port: Default::default(),
                port_drag_cursor: Cell::new(Point::new(0.0, 0.0)),
                zoom_gesture_initial_zoom: Default::default(),
                zoom_gesture_anchor: Default::default(),
                move_view_state: Default::default(),
                highlighted_nodes: Default::default(),
                highlighted_edges: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GraphView {
        const NAME: &'static str = "HelvumGraphView";
        type Type = super::GraphView;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("graphview");
        }
    }

    impl ObjectImpl for GraphView {
        fn constructed(&self) {
            self.parent_constructed();

            self.obj().add_css_class("view");

            self.obj().set_overflow(gtk::Overflow::Hidden);

            self.setup_node_dragging();
            self.setup_port_drag_and_drop();
            self.setup_scroll_zooming();
            self.setup_zoom_gesture();
            self.setup_move_view();
            self.setup_path_highlighting();
        }

        fn dispose(&self) {
            for nw in self.graph.borrow().node_weights() {
                nw.widget.unparent();
            }
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
                vec![
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vadjustment"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("hscroll-policy"),
                    glib::ParamSpecOverride::for_interface::<gtk::Scrollable>("vscroll-policy"),
                    glib::ParamSpecDouble::builder("zoom-factor")
                        .minimum(0.3)
                        .maximum(4.0)
                        .default_value(1.0)
                        .flags(glib::ParamFlags::CONSTRUCT | glib::ParamFlags::READWRITE)
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "hadjustment" => self.hadjustment.borrow().to_value(),
                "vadjustment" => self.vadjustment.borrow().to_value(),
                "hscroll-policy" | "vscroll-policy" => gtk::ScrollablePolicy::Natural.to_value(),
                "zoom-factor" => self.zoom_factor.get().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "hadjustment" => {
                    self.set_adjustment(&obj, value.get().ok(), gtk::Orientation::Horizontal)
                }
                "vadjustment" => {
                    self.set_adjustment(&obj, value.get().ok(), gtk::Orientation::Vertical)
                }
                "hscroll-policy" | "vscroll-policy" => {}
                "zoom-factor" => {
                    self.zoom_factor.set(value.get().unwrap());
                    obj.queue_allocate();
                }
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for GraphView {
        fn size_allocate(&self, _width: i32, _height: i32, baseline: i32) {
            let widget = &*self.obj();

            for nw in self.graph.borrow().node_weights() {
                let node = &nw.widget;
                let point = &nw.position;
                let (_, natural_size) = node.preferred_size();

                let transform = self
                    .canvas_space_to_screen_space_transform()
                    .translate(point);

                node.allocate(
                    natural_size.width(),
                    natural_size.height(),
                    baseline,
                    Some(transform),
                );
            }

            if let Some(ref hadjustment) = *self.hadjustment.borrow() {
                self.set_adjustment_values(widget, hadjustment, gtk::Orientation::Horizontal);
            }
            if let Some(ref vadjustment) = *self.vadjustment.borrow() {
                self.set_adjustment_values(widget, vadjustment, gtk::Orientation::Vertical);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = &*self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);

            let inv_transform = self.screen_space_to_canvas_space_transform();

            let p0 = inv_transform.transform_point(&Point::new(0.0, 0.0));
            let p1 = inv_transform.transform_point(&Point::new(width, height));

            let (min_x, max_x) = (p0.x().min(p1.x()), p0.x().max(p1.x()));
            let (min_y, max_y) = (p0.y().min(p1.y()), p0.y().max(p1.y()));

            let highlighted_nodes = self.highlighted_nodes.borrow();
            let is_any_highlighted = !highlighted_nodes.is_empty();

            // Draw all visible children
            for (idx, nw) in self.graph.borrow().node_references() {
                let node = &nw.widget;
                let point = &nw.position;

                let n_width = node.width() as f32;
                let n_height = node.height() as f32;

                let is_visible = point.x() < max_x
                    && point.y() < max_y
                    && point.x() + n_width > min_x
                    && point.y() + n_height > min_y;

                if is_visible {
                    let is_highlighted = !is_any_highlighted || highlighted_nodes.contains(&idx);
                    let is_online = node.online();

                    if !is_online {
                        snapshot.push_opacity(0.15);
                        widget.snapshot_child(node, snapshot);
                        snapshot.pop();
                    } else if is_highlighted {
                        widget.snapshot_child(node, snapshot);
                    } else {
                        snapshot.push_opacity(0.3);
                        widget.snapshot_child(node, snapshot);
                        snapshot.pop();
                    }
                }
            }

            self.snapshot_links(widget, snapshot, min_x, max_x, min_y, max_y);
        }
    }

    impl ScrollableImpl for GraphView {}
 
    impl GraphView {
        /// Returns a [`gsk::Transform`] matrix that can translate from canvas space to screen space.
        ///
        /// Canvas space is non-zoomed, and (0, 0) is fixed at the middle of the graph. \
        /// Screen space is zoomed and adjusted for scrolling, (0, 0) is at the top-left corner of the window.
        ///
        /// This is the inverted form of [`Self::screen_space_to_canvas_space_transform()`].
        fn canvas_space_to_screen_space_transform(&self) -> gsk::Transform {
            let hadj = self.hadjustment.borrow().as_ref().unwrap().value();
            let vadj = self.vadjustment.borrow().as_ref().unwrap().value();
            let zoom_factor = self.zoom_factor.get();

            gsk::Transform::new()
                .translate(&Point::new(-hadj as f32, -vadj as f32))
                .scale(zoom_factor as f32, zoom_factor as f32)
        }

        /// Returns a [`gsk::Transform`] matrix that can translate from screen space to canvas space.
        ///
        /// This is the inverted form of [`Self::canvas_space_to_screen_space_transform()`], see that function for a more detailed explantion.
        fn screen_space_to_canvas_space_transform(&self) -> gsk::Transform {
            self.canvas_space_to_screen_space_transform()
                .invert()
                .unwrap()
        }

        fn setup_node_dragging(&self) {
            let drag_controller = gtk::GestureDrag::new();

            drag_controller.connect_drag_begin(|drag_controller, x, y| {
                let widget = drag_controller
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::GraphView>()
                    .expect("drag-begin event is not on the GraphView");
                let mut dragged_node = widget.imp().dragged_node.borrow_mut();

                // pick() should at least return the widget itself.
                let target = widget
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .expect("drag-begin pick() did not return a widget");
                *dragged_node = if target.ancestor(Port::static_type()).is_some() {
                    // The user targeted a port, so the dragging should be handled by the Port
                    // component instead of here.
                    None
                } else if let Some(target) = target.ancestor(Node::static_type()) {
                    // The user targeted a Node without targeting a specific Port.
                    // Drag the Node around the screen.
                    let node = target.dynamic_cast_ref::<Node>().unwrap();

                    let Some(canvas_node_pos) = widget.node_position(node) else {
                        return;
                    };
                    let canvas_cursor_pos = widget
                        .imp()
                        .screen_space_to_canvas_space_transform()
                        .transform_point(&Point::new(x as f32, y as f32));

                    Some(DragState {
                        node: node.clone().downgrade(),
                        offset: Point::new(
                            canvas_cursor_pos.x() - canvas_node_pos.x(),
                            canvas_cursor_pos.y() - canvas_node_pos.y(),
                        ),
                    })
                } else {
                    None
                }
            });
            drag_controller.connect_drag_update(|drag_controller, x, y| {
                let widget = drag_controller
                    .widget()
                    .unwrap()
                    .dynamic_cast::<super::GraphView>()
                    .expect("drag-update event is not on the GraphView");
                let dragged_node = widget.imp().dragged_node.borrow();
                let Some(DragState { node, offset }) = dragged_node.as_ref() else {
                    return;
                };
                let Some(node) = node.upgrade() else { return };

                let (start_x, start_y) = drag_controller
                    .start_point()
                    .expect("Drag has no start point");

                let onscreen_node_origin = Point::new((start_x + x) as f32, (start_y + y) as f32);
                let transform = widget.imp().screen_space_to_canvas_space_transform();
                let canvas_node_origin = transform.transform_point(&onscreen_node_origin);

                widget.move_node(
                    &node,
                    &Point::new(
                        canvas_node_origin.x() - offset.x(),
                        canvas_node_origin.y() - offset.y(),
                    ),
                );
            });
            self.obj().add_controller(drag_controller);
        }

        fn setup_port_drag_and_drop(&self) {
            let controller = gtk::DropControllerMotion::new();

            controller.connect_enter(|controller, x, y| {
                let graph = controller
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .expect("Widget should be a graphview");

                graph.imp().port_drag_enter(controller, x, y)
            });

            controller.connect_motion(|controller, x, y| {
                let graph = controller
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .expect("Widget should be a graphview");

                graph.imp().port_drag_motion(x, y)
            });

            controller.connect_leave(|controller| {
                let graph = controller
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .expect("Widget should be a graphview");

                graph.imp().port_drag_leave()
            });

            self.obj().add_controller(controller);
        }

        fn port_drag_enter(&self, controller: &gtk::DropControllerMotion, x: f64, y: f64) {
            let Some(drop) = controller.drop() else {
                return;
            };

            self.port_drag_cursor.set(Point::new(x as f32, y as f32));

            drop.read_value_async(
                Port::static_type(),
                glib::Priority::DEFAULT,
                Option::<&gio::Cancellable>::None,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |value| {
                        let Ok(value) = value else {
                            return;
                        };
                        let port: &Port = value.get().expect("Value should contain a port");

                        imp.dragged_port.set(Some(port));
                    }
                ),
            );

            self.obj().queue_draw();
        }

        fn port_drag_motion(&self, x: f64, y: f64) {
            if self.dragged_port.upgrade().is_some() {
                self.port_drag_cursor.set(Point::new(x as f32, y as f32));

                self.obj().queue_draw();
            }
        }

        fn port_drag_leave(&self) {
            if self.dragged_port.upgrade().is_some() {
                self.dragged_port.set(None);
                self.obj().queue_draw();
            }
        }

        fn setup_scroll_zooming(&self) {
            // We're only interested in the vertical axis, but for devices like touchpads,
            // not capturing a small accidental horizontal move may cause the scroll to be disrupted if a widget
            // higher up captures it instead.
            let scroll_controller =
                gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);

            scroll_controller.connect_scroll(|eventcontroller, _, delta_y| {
                let event = eventcontroller.current_event().unwrap(); // We are inside the event handler, so it must have an event

                if event
                    .modifier_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    let widget = eventcontroller
                        .widget()
                        .unwrap()
                        .downcast::<super::GraphView>()
                        .unwrap();
                    widget.set_zoom_factor(widget.zoom_factor() + (0.1 * -delta_y), None);

                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            self.obj().add_controller(scroll_controller);
        }

        fn setup_zoom_gesture(&self) {
            let zoom_gesture = gtk::GestureZoom::new();
            zoom_gesture.connect_begin(|gesture, _| {
                let widget = gesture
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .unwrap();

                widget
                    .imp()
                    .zoom_gesture_initial_zoom
                    .set(Some(widget.zoom_factor()));
                widget
                    .imp()
                    .zoom_gesture_anchor
                    .set(gesture.bounding_box_center());
            });
            zoom_gesture.connect_scale_changed(move |gesture, delta| {
                let widget = gesture
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .unwrap();

                let initial_zoom = widget
                    .imp()
                    .zoom_gesture_initial_zoom
                    .get()
                    .expect("Initial zoom not set during zoom gesture");

                widget.set_zoom_factor(initial_zoom * delta, gesture.bounding_box_center());
            });
            self.obj().add_controller(zoom_gesture);
        }

        fn setup_move_view(&self) {
            let drag_controller = gtk::GestureDrag::new();

            drag_controller.set_button(gtk::gdk::BUTTON_MIDDLE);

            // TODO: set `all-scroll` cursor while dragging view

            drag_controller.connect_drag_begin(|drag_controller, _, _| {
                let widget = drag_controller
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .unwrap();

                widget.imp().move_view_state.set((0.0, 0.0));
            });

            drag_controller.connect_drag_update(|drag_controller, x, y| {
                let widget = drag_controller
                    .widget()
                    .unwrap()
                    .downcast::<super::GraphView>()
                    .unwrap();

                let imp = widget.imp();
                let state = imp.move_view_state.replace((x, y));
                let delta_x = state.0 - x;
                let delta_y = state.1 - y;

                let hadjustment_ref = imp.hadjustment.borrow();
                let vadjustment_ref = imp.vadjustment.borrow();
                let hadjustment = hadjustment_ref.as_ref().unwrap();
                let vadjustment = vadjustment_ref.as_ref().unwrap();

                let new_hadjustment = hadjustment.value() + delta_x;
                let new_vadjustment = vadjustment.value() + delta_y;

                hadjustment.set_value(new_hadjustment);
                vadjustment.set_value(new_vadjustment);
            });

            self.obj().add_controller(drag_controller);
        }

        fn setup_path_highlighting(&self) {
            let obj = self.obj();
            let motion_controller = gtk::EventControllerMotion::new();

            motion_controller.connect_motion(glib::clone!(
                #[weak]
                obj,
                move |_, x, y| {
                    obj.update_highlighting(x, y);
                }
            ));

            motion_controller.connect_leave(glib::clone!(
                #[weak]
                obj,
                move |_| {
                    obj.clear_highlighting();
                }
            ));

            obj.add_controller(motion_controller);
        }

        fn draw_link(
            &self,
            snapshot: &gtk::Snapshot,
            output_anchor: &Point,
            input_anchor: &Point,
            active: bool,
            online: bool,
            color: &gdk::RGBA,
        ) {
            let output_x = output_anchor.x();
            let output_y = output_anchor.y();
            let input_x = input_anchor.x();
            let input_y = input_anchor.y();

            let zoom = self.zoom_factor.get() as f32;

            let builder = gsk::PathBuilder::new();
            builder.move_to(output_x, output_y);

            // If the output port is farther right than the input port and they have
            // a similar y coordinate, apply a y offset to the control points
            // so that the curve sticks out a bit.
            let y_control_offset = if output_x > input_x {
                f32::max(0.0, (25.0 * zoom) - (output_y - input_y).abs())
            } else {
                0.0
            };

            // Place curve control offset by half the x distance between the two points.
            // This makes the curve scale well for varying distances between the two ports,
            // especially when the output port is farther right than the input port.
            let half_x_dist = f32::abs(output_x - input_x) / 2.0;
            builder.cubic_to(
                output_x + half_x_dist,
                output_y - y_control_offset,
                input_x - half_x_dist,
                input_y - y_control_offset,
                input_x,
                input_y,
            );

            let path = builder.to_path();

            let color = if online {
                color.clone()
            } else {
                gdk::RGBA::new(0.5, 0.5, 0.5, 1.0)
            };

            let stroke_width = f32::max(1.0, 2.0 * zoom);
            let stroke = gsk::Stroke::new(stroke_width);

            // Use dashed line for inactive or offline links, full line otherwise.
            if !active || !online {
                stroke.set_dash(&[10.0 * zoom, 5.0 * zoom]);
            }

            snapshot.append_stroke(&path, &stroke, &color);
        }

        fn draw_dragged_link(&self, port: &Port, snapshot: &gtk::Snapshot, colors: &Colors) {
            let Some(port_anchor) = port.compute_point(&*self.obj(), &port.link_anchor()) else {
                return;
            };
            let drag_cursor = self.port_drag_cursor.get();

            /* If we can find a linkable port under the cursor, link to its anchor,
             * otherwise link to the mouse cursor */
            let picked_port = self
                .obj()
                .pick(
                    drag_cursor.x().into(),
                    drag_cursor.y().into(),
                    gtk::PickFlags::DEFAULT,
                )
                .and_then(|widget| widget.ancestor(Port::static_type()).and_downcast::<Port>())
                .filter(|picked_port| port.is_linkable_to(picked_port));
            let picked_port_anchor = picked_port.and_then(|picked_port| {
                picked_port.compute_point(&*self.obj(), &picked_port.link_anchor())
            });
            let other_anchor = picked_port_anchor.unwrap_or(drag_cursor);

            let (output_anchor, input_anchor) = match port.port_direction() {
                PortDirection::Output => (&port_anchor, &other_anchor),
                PortDirection::Input => (&other_anchor, &port_anchor),
            };

            let mut media_type = MediaType::from(port.media_type());
            if media_type == MediaType::Unknown {
                media_type = MediaType::Unknown;
            }

            let color = &colors.color_for_media_type(media_type);

            self.draw_link(snapshot, output_anchor, input_anchor, false, true, color);
        }

        fn snapshot_links(
            &self,
            _widget: &super::GraphView,
            snapshot: &gtk::Snapshot,
            min_x: f32,
            max_x: f32,
            min_y: f32,
            max_y: f32,
        ) {
            let colors = Colors {
                audio: gdk::RGBA::new(50.0 / 255.0, 100.0 / 255.0, 240.0 / 255.0, 1.0),
                video: gdk::RGBA::new(200.0 / 255.0, 200.0 / 255.0, 0.0, 1.0),
                midi: gdk::RGBA::new(200.0 / 255.0, 0.0, 50.0 / 255.0, 1.0),
                unknown: gdk::RGBA::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0),
            };

            let graph = self.graph.borrow();
            let highlighted_edges = self.highlighted_edges.borrow();
            let is_any_highlighted = !self.highlighted_nodes.borrow().is_empty();

            for edge in graph.edge_references() {
                let link = edge.weight();

                if link.pending_check() {
                    continue;
                }

                let (source, target) = graph.edge_endpoints(edge.id()).unwrap();
                let source_nw = &graph[source];
                let target_nw = &graph[target];

                let padding = 100.0;
                let l_min_x = source_nw.position.x().min(target_nw.position.x()) - padding;
                let l_max_x = source_nw.position.x().max(target_nw.position.x()) + 500.0;
                let l_min_y = source_nw.position.y().min(target_nw.position.y()) - padding;
                let l_max_y = source_nw.position.y().max(target_nw.position.y()) + 500.0;

                if l_max_x < min_x || l_min_x > max_x || l_max_y < min_y || l_min_y > max_y {
                    continue;
                }

                let mut media_type = MediaType::from(link.media_type());

                if media_type == MediaType::Unknown {
                    if let Some(output_port) = link.output_port() {
                        media_type = MediaType::from(output_port.media_type());
                    } else if let Some(input_port) = link.input_port() {
                        media_type = MediaType::from(input_port.media_type());
                    }
                }

                let color = &colors.color_for_media_type(media_type);

                let Some((output_anchor, input_anchor)) = self.get_link_coordinates(link) else {
                    warn!("Could not get allocation of ports of link: {:?}", link);
                    continue;
                };

                let is_highlighted = !is_any_highlighted || highlighted_edges.contains(&edge.id());
                if is_highlighted {
                    self.draw_link(
                        snapshot,
                        &output_anchor,
                        &input_anchor,
                        link.active(),
                        link.online(),
                        color,
                    );
                } else {
                    snapshot.push_opacity(0.3);
                    self.draw_link(
                        snapshot,
                        &output_anchor,
                        &input_anchor,
                        link.active(),
                        link.online(),
                        color,
                    );
                    snapshot.pop();
                }
            }

            if let Some(port) = self.dragged_port.upgrade() {
                self.draw_dragged_link(&port, snapshot, &colors);
            }
        }

        /// Get coordinates for the drawn link to start at and to end at.
        ///
        /// # Returns
        /// `Some((output_anchor, input_anchor))` if all objects the links refers to exist as widgets
        /// and those widgets are contained by the graph.
        ///
        /// The returned coordinates are in screen-space of the graph.
        fn get_link_coordinates(&self, link: &Link) -> Option<(graphene::Point, graphene::Point)> {
            let widget = &*self.obj();

            let output_port = link.output_port()?;
            let output_anchor = output_port.compute_point(widget, &output_port.link_anchor())?;

            let input_port = link.input_port()?;
            let input_anchor = input_port.compute_point(widget, &input_port.link_anchor())?;

            Some((output_anchor, input_anchor))
        }

        fn set_adjustment(
            &self,
            obj: &super::GraphView,
            adjustment: Option<&gtk::Adjustment>,
            orientation: gtk::Orientation,
        ) {
            match orientation {
                gtk::Orientation::Horizontal => {
                    *self.hadjustment.borrow_mut() = adjustment.cloned()
                }
                gtk::Orientation::Vertical => *self.vadjustment.borrow_mut() = adjustment.cloned(),
                _ => unimplemented!(),
            }

            if let Some(adjustment) = adjustment {
                adjustment.connect_value_changed(clone!(
                    #[weak]
                    obj,
                    move |_| obj.queue_allocate()
                ));
            }
        }

        fn set_adjustment_values(
            &self,
            obj: &super::GraphView,
            adjustment: &gtk::Adjustment,
            orientation: gtk::Orientation,
        ) {
            let size = match orientation {
                gtk::Orientation::Horizontal => obj.width(),
                gtk::Orientation::Vertical => obj.height(),
                _ => unimplemented!(),
            };
            let zoom_factor = self.zoom_factor.get();

            adjustment.configure(
                adjustment.value(),
                -(CANVAS_SIZE / 2.0) * zoom_factor,
                (CANVAS_SIZE / 2.0) * zoom_factor,
                (f64::from(size) * 0.1) * zoom_factor,
                (f64::from(size) * 0.9) * zoom_factor,
                f64::from(size) * zoom_factor,
            );
        }
    }
}

glib::wrapper! {
    pub struct GraphView(ObjectSubclass<imp::GraphView>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl GraphView {
    pub(crate) fn graph(&self) -> &RefCell<StableGraph<NodeWeight, Link, Directed>> {
        &self.imp().graph
    }
}

impl GraphView {
    pub const ZOOM_MIN: f64 = 0.3;
    pub const ZOOM_MAX: f64 = 4.0;

    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn link_count(&self) -> usize {
        self.imp().graph.borrow().edge_count()
    }

    pub fn fit_to_content(&self) {
        let imp = self.imp();
        let graph = imp.graph.borrow();
        
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        
        let mut has_nodes = false;
        
        for nw in graph.node_weights() {
            has_nodes = true;
            let x = nw.position.x();
            let y = nw.position.y();
            let (_, nat) = nw.widget.preferred_size();
            let w = nat.width() as f32;
            let h = nat.height() as f32;
            
            if x < min_x { min_x = x; }
            if y < min_y { min_y = y; }
            if x + w > max_x { max_x = x + w; }
            if y + h > max_y { max_y = y + h; }
        }
        
        if !has_nodes {
            return;
        }
        
        let padding = 100.0;
        min_x -= padding;
        min_y -= padding;
        max_x += padding;
        max_y += padding;
        
        let content_w = (max_x - min_x) as f64;
        let content_h = (max_y - min_y) as f64;
        
        let alloc_w = self.width() as f64;
        let alloc_h = self.height() as f64;
        
        if alloc_w <= 0.0 || alloc_h <= 0.0 {
            return;
        }
        
        let zoom_x = alloc_w / content_w;
        let zoom_y = alloc_h / content_h;
        
        let target_zoom = zoom_x.min(zoom_y).min(1.0).max(0.3);
        
        self.set_property("zoom-factor", target_zoom);
        
        let center_x = (min_x as f64 + max_x as f64) / 2.0;
        let center_y = (min_y as f64 + max_y as f64) / 2.0;
        
        let hadj_val = center_x * target_zoom - alloc_w / 2.0;
        let vadj_val = center_y * target_zoom - alloc_h / 2.0;
        
        if let Some(adj) = imp.hadjustment.borrow().as_ref() {
            adj.set_value(hadj_val);
        }
        if let Some(adj) = imp.vadjustment.borrow().as_ref() {
            adj.set_value(vadj_val);
        }
    }

    pub fn zoom_factor(&self) -> f64 {
        self.property("zoom-factor")
    }

    /// Set the scale factor.
    ///
    /// A factor of 1.0 is equivalent to 100% zoom, 0.5 to 50% zoom etc.
    ///
    /// An optional anchor (in canvas-space coordinates) can be specified, which will be used as the center of the zoom,
    /// so that its position stays fixed.
    /// If no anchor is specified, the middle of the screen is used instead.
    ///
    /// Note that the zoom level is [clamped](`f64::clamp`) to between 30% and 300%.
    /// See [`Self::ZOOM_MIN`] and [`Self::ZOOM_MAX`].
    pub fn set_zoom_factor(&self, zoom_factor: f64, anchor: Option<(f64, f64)>) {
        let zoom_factor = zoom_factor.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);

        let (anchor_x_screen, anchor_y_screen) =
            anchor.unwrap_or_else(|| (self.width() as f64 / 2.0, self.height() as f64 / 2.0));

        let old_zoom = self.imp().zoom_factor.get();
        let hadjustment_ref = self.imp().hadjustment.borrow();
        let vadjustment_ref = self.imp().vadjustment.borrow();
        let hadjustment = hadjustment_ref.as_ref().unwrap();
        let vadjustment = vadjustment_ref.as_ref().unwrap();

        let x_total = (anchor_x_screen + hadjustment.value()) / old_zoom;
        let y_total = (anchor_y_screen + vadjustment.value()) / old_zoom;

        let new_hadjustment = x_total * zoom_factor - anchor_x_screen;
        let new_vadjustment = y_total * zoom_factor - anchor_y_screen;

        hadjustment.set_value(new_hadjustment);
        vadjustment.set_value(new_vadjustment);

        self.set_property("zoom-factor", zoom_factor);
    }

    pub fn add_node(&self, node: Node, node_type: Option<NodeType>) {
        let imp = self.imp();
        node.set_parent(self);

        // Place widgets in colums of 3, growing down
        let x = if let Some(node_type) = node_type {
            match node_type {
                NodeType::Output => 20.0,
                NodeType::Input => 820.0,
            }
        } else {
            420.0
        };

        let mut y = 20.0;
        let mut found_spot = false;
        
        let margin = 40.0;
        let (_, nat_size) = node.preferred_size();
        let current_width = nat_size.width() as f32;
        let current_height = (nat_size.height() as f32).max(150.0);

        while !found_spot {
            found_spot = true;
            for nw in imp.graph.borrow().node_weights() {
                let nw_x = nw.position.x();
                let nw_y = nw.position.y();
                let (_, nw_nat) = nw.widget.preferred_size();
                let nw_width = nw_nat.width() as f32;
                let nw_height = (nw_nat.height() as f32).max(150.0);

                if x < nw_x + nw_width + margin && x + current_width + margin > nw_x &&
                   y < nw_y + nw_height + margin && y + current_height + margin > nw_y {
                    y = nw_y + nw_height + margin;
                    found_spot = false;
                    break;
                }
            }
        }

        let mut graph = imp.graph.borrow_mut();
        let mut node_to_index = imp.node_to_index.borrow_mut();

        let position = Point::new(x, y);
        let index = graph.add_node(crate::ui::graph::graph_view::NodeWeight {
            widget: node.clone(),
            position,
        });
        node_to_index.insert(node, index);
    }

    pub fn remove_node(&self, node: &Node) {
        let imp = self.imp();
        let mut graph = imp.graph.borrow_mut();
        let mut node_to_index = imp.node_to_index.borrow_mut();
        let mut link_to_index = imp.link_to_index.borrow_mut();

        if let Some(index) = node_to_index.remove(node) {
            // Remove all associated links from the link_to_index map
            let edge_indices: Vec<_> = graph.edges(index).map(|e| e.id()).collect();
            for edge_idx in edge_indices {
                let link = graph.edge_weight(edge_idx).unwrap().clone();
                link_to_index.remove(link.upcast_ref::<glib::Object>());
                link.unparent();
            }

            graph.remove_node(index);
            node.unparent();
        } else {
            log::warn!("Tried to remove non-existant node widget from graph");
        }
    }

    pub fn add_link(&self, link: Link) {
        link.connect_notify_local(
            Some("active"),
            glib::clone!(
                #[weak(rename_to = graph)]
                self,
                move |_, _| {
                    graph.queue_draw();
                }
            ),
        );
        link.connect_notify_local(
            Some("media-type"),
            glib::clone!(
                #[weak(rename_to = graph)]
                self,
                move |_, _| {
                    graph.queue_draw();
                }
            ),
        );
        let output_port = link.output_port().expect("Link should have an output port");
        let input_port = link.input_port().expect("Link should have an input port");

        let Some(output_node) = output_port
            .ancestor(Node::static_type())
            .and_downcast::<Node>() else {
                log::warn!("Output port has no node ancestor");
                return;
            };
        let Some(input_node) = input_port
            .ancestor(Node::static_type())
            .and_downcast::<Node>() else {
                log::warn!("Input port has no node ancestor");
                return;
            };

        let imp = self.imp();
        let mut graph = imp.graph.borrow_mut();
        let node_to_index = imp.node_to_index.borrow();
        let mut link_to_index = imp.link_to_index.borrow_mut();

        let Some(&output_idx) = node_to_index.get(&output_node) else {
            log::warn!("Output node not found in graph");
            return;
        };
        let Some(&input_idx) = node_to_index.get(&input_node) else {
            log::warn!("Input node not found in graph");
            return;
        };

        let edge_idx = graph.add_edge(output_idx, input_idx, link.clone());
        link_to_index.insert(link.clone(), edge_idx);
        
        link.set_parent(self);

        self.queue_draw();
    }

    pub fn remove_link(&self, link: &Link) {
        let imp = self.imp();
        let mut graph = imp.graph.borrow_mut();
        let mut link_to_index = imp.link_to_index.borrow_mut();

        if let Some(index) = link_to_index.remove(link.upcast_ref::<glib::Object>()) {
            graph.remove_edge(index);
            link.unparent();
        }

        self.queue_draw();
    }

    pub fn clear(&self) {
        let imp = self.imp();
        let mut graph = imp.graph.borrow_mut();
        let mut node_to_index = imp.node_to_index.borrow_mut();
        let mut link_to_index = imp.link_to_index.borrow_mut();

        for nw in graph.node_weights() {
            nw.widget.unparent();
        }
        for link in link_to_index.keys() {
            link.unparent();
        }

        graph.clear();
        node_to_index.clear();
        link_to_index.clear();

        self.queue_draw();
    }

    /// Get the position of the specified node inside the graphview.
    ///
    /// The returned position is in canvas-space (non-zoomed, (0, 0) fixed in the middle of the canvas).
    pub(super) fn node_position(&self, node: &Node) -> Option<Point> {
        let imp = self.imp();
        let graph = imp.graph.borrow();
        let node_to_index = imp.node_to_index.borrow();

        node_to_index.get(node).map(|&idx| graph[idx].position)
    }

    pub(super) fn move_node(&self, widget: &Node, point: &Point) {
        let imp = self.imp();
        let mut graph = imp.graph.borrow_mut();
        let node_to_index = imp.node_to_index.borrow();

        let Some(&idx) = node_to_index.get(widget) else {
            log::warn!("Node is not on the graph");
            return;
        };
        let nw = &mut graph[idx];

        // Clamp the new position to within the graph, so a node can't be moved outside it and be lost.
        nw.position.set_x(point.x().clamp(
            -(CANVAS_SIZE / 2.0) as f32,
            (CANVAS_SIZE / 2.0) as f32 - widget.width() as f32,
        ));
        nw.position.set_y(point.y().clamp(
            -(CANVAS_SIZE / 2.0) as f32,
            (CANVAS_SIZE / 2.0) as f32 - widget.height() as f32,
        ));

        self.queue_allocate();
    }

    pub fn update_highlighting(&self, x: f64, y: f64) {
        let imp = self.imp();

        let target = self.pick(x, y, gtk::PickFlags::DEFAULT);
        let node_widget = target
            .and_then(|t| t.ancestor(Node::static_type()))
            .and_downcast::<Node>();

        let mut highlighted_nodes = imp.highlighted_nodes.borrow_mut();
        let mut highlighted_edges = imp.highlighted_edges.borrow_mut();

        if let Some(ref node) = node_widget {
            let node_to_index = imp.node_to_index.borrow();
            if let Some(&idx) = node_to_index.get(node) {
                if !highlighted_nodes.is_empty() && highlighted_nodes.contains(&idx) {
                    return;
                }
            }
        } else if highlighted_nodes.is_empty() {
            return;
        }

        highlighted_nodes.clear();
        highlighted_edges.clear();

        if let Some(node) = node_widget {
            let graph = imp.graph.borrow();
            let node_to_index = imp.node_to_index.borrow();

            if let Some(&start_idx) = node_to_index.get(&node) {
                // Downstream
                let mut bfs = Bfs::new(&*graph, start_idx);
                while let Some(nx) = bfs.next(&*graph) {
                    highlighted_nodes.insert(nx);
                }

                // Upstream
                let rev_graph = Reversed(&*graph);
                let mut bfs_rev = Bfs::new(rev_graph, start_idx);
                while let Some(nx) = bfs_rev.next(rev_graph) {
                    highlighted_nodes.insert(nx);
                }

                // Collect all edges between highlighted nodes
                for edge in graph.edge_references() {
                    if highlighted_nodes.contains(&edge.source())
                        && highlighted_nodes.contains(&edge.target())
                    {
                        highlighted_edges.insert(edge.id());
                    }
                }
            }
        }

        self.queue_draw();
    }

    pub fn clear_highlighting(&self) {
        let imp = self.imp();
        if !imp.highlighted_nodes.borrow().is_empty() {
            imp.highlighted_nodes.borrow_mut().clear();
            imp.highlighted_edges.borrow_mut().clear();
            self.queue_draw();
        }
    }
}

impl Default for GraphView {
    fn default() -> Self {
        Self::new()
    }
}

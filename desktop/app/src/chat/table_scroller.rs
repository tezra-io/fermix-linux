//! A table that wraps to the width it is given and scrolls sideways only below
//! its columns' floors. A `ScrolledWindow` that can scroll sideways measures its
//! content's height at the content's narrowest width, which for wrapping cells
//! is far taller than they draw, so a table in one alone leaves a gap under it.
//! This widget holds the scroller and asks the grid for its height at the width
//! the grid will really have.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

glib::wrapper! {
    pub struct TableScroller(ObjectSubclass<imp::TableScroller>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TableScroller {
    pub fn new(grid: &gtk::Grid) -> TableScroller {
        let widget: TableScroller = glib::Object::builder().property("hexpand", true).build();
        let scroller = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_width(true)
            .child(grid)
            .build();
        scroller.set_parent(&widget);
        let imp = widget.imp();
        imp.scroller.replace(Some(scroller));
        imp.grid.replace(Some(grid.clone()));
        widget
    }
}

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct TableScroller {
        pub scroller: RefCell<Option<gtk::ScrolledWindow>>,
        pub grid: RefCell<Option<gtk::Grid>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TableScroller {
        const NAME: &'static str = "FermixTableScroller";
        type Type = super::TableScroller;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TableScroller {
        fn dispose(&self) {
            if let Some(scroller) = self.scroller.take() {
                scroller.unparent();
            }
            self.grid.take();
        }
    }

    impl WidgetImpl for TableScroller {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        /// Sideways, the scroller's own measure: it shrinks below the grid and
        /// scrolls. Downwards, the grid's height at the width it will be given,
        /// which is never below its narrowest.
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let scroller = self.scroller.borrow();
            let grid = self.grid.borrow();
            let (Some(scroller), Some(grid)) = (scroller.as_ref(), grid.as_ref()) else {
                return (0, 0, -1, -1);
            };
            if orientation == gtk::Orientation::Horizontal {
                return scroller.measure(orientation, for_size);
            }
            let (narrowest, natural, _, _) = grid.measure(gtk::Orientation::Horizontal, -1);
            let width = if for_size < 0 {
                natural
            } else {
                for_size.max(narrowest)
            };
            let (height, _, _, _) = grid.measure(orientation, width);
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(scroller) = self.scroller.borrow().as_ref() {
                scroller.allocate(width, height, baseline, None);
            }
        }
    }
}

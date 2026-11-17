//! The Objects dialog: the rows of the print's objects and the bed map they lie on. The outlines are built again only
//! when the objects or the bed change, not with every status: a slicer outline can have hundreds of points.

use crate::items::replace_rows;
use crate::{BedMapItem, PrintObjectItem, Printer, bed_map};
use sailhand_core::sources::klipper::{Bed, BedPoint, PrintObject};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// What the outlines were built from.
#[derive(Default, PartialEq)]
struct Geometry {
    bed: Option<Bed>,
    places: Vec<(Vec<BedPoint>, Option<BedPoint>)>,
}

impl Geometry {
    fn of(objects: &[PrintObject], bed: Option<Bed>) -> Self {
        Self { bed, places: objects.iter().map(|object| (object.outline.clone(), object.center)).collect() }
    }
}

#[derive(Default)]
pub struct ObjectsView {
    rows: Rc<VecModel<PrintObjectItem>>,
    geometry: RefCell<Geometry>,
    outlines: RefCell<Vec<SharedString>>,
}

impl ObjectsView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_print_objects(ModelRc::from(self.rows.clone()));
    }

    pub fn show(&self, printer: &Printer, objects: &[PrintObject], bed: Option<Bed>) {
        let geometry = Geometry::of(objects, bed);
        if *self.geometry.borrow() != geometry {
            self.build_map(printer, objects, bed);
            *self.geometry.borrow_mut() = geometry;
        }
        let outlines = self.outlines.borrow();
        let rows = objects
            .iter()
            .zip(outlines.iter())
            .map(|(object, outline)| PrintObjectItem {
                name: object.name.as_str().into(),
                excluded: object.excluded,
                printing: object.printing,
                outline: outline.clone(),
            })
            .collect();
        replace_rows(&self.rows, rows);
    }

    fn build_map(&self, printer: &Printer, objects: &[PrintObject], bed: Option<Bed>) {
        let map = bed.filter(|_| bed_map::shows_map(objects, bed));
        *self.outlines.borrow_mut() = objects
            .iter()
            .map(|object| map.map(|bed| bed_map::outline(object, bed)).unwrap_or_default().into())
            .collect();
        printer.set_bed_map(match map {
            Some(bed) => BedMapItem {
                shown: true,
                width: bed.width() as f32,
                depth: bed.depth() as f32,
                grid: bed_map::grid(bed).into(),
            },
            None => BedMapItem::default(),
        });
    }
}

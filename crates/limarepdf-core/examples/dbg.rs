use limarepdf_core::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let base = Pdf::open(&a[1], None).unwrap();
    for pg in 0..base.page_count().min(2) {
        let n = base.page_objects(pg).unwrap().len();
        for id in 0..n {
            let mut p = Pdf::open(&a[1], None).unwrap();
            let o = p.page_objects(pg).unwrap()[id].clone();
            let b = o.bbox;
            let r = p.set_object_bbox(pg, id, [b[0] + 30.0, b[1] + 20.0, b[2] + 60.0, b[3] + 40.0]);
            let after = p.page_objects(pg).unwrap();
            let nb = after.get(id).map(|x| x.bbox);
            println!(
                "p{pg} #{id} {:?} move={:?} n={}->{} newbox={:?} oldbox={:?}",
                o.kind,
                r.is_ok(),
                n,
                after.len(),
                nb,
                b
            );
            let mut p = Pdf::open(&a[1], None).unwrap();
            let r2 = if o.kind == ObjectKind::Text {
                p.edit_text(
                    pg,
                    id,
                    &TextEdit {
                        text: "Edited".into(),
                        font: None,
                        size: None,
                        color: None,
                        parts: Vec::new(),
                    },
                )
                .map(|_| ())
            } else {
                p.delete_object(pg, id)
            };
            let ok_render = {
                let bytes = p.to_bytes().unwrap();
                limarepdf_core::render::load(bytes, None).is_ok()
            };
            println!(
                "    edit/delete={:?} render_ok={ok_render} n={}",
                r2.map_err(|e| e.to_string()),
                p.page_objects(pg).unwrap().len()
            );
        }
    }
}

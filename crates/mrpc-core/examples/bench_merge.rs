//! Temporary benchmark (run then delete): merge two large synthetic packs and
//! report the wall-clock time. Validates the pre-serialised HashSet dedup in
//! `ResourcePack::merge_from`.

use mrpc_core::pack::ResourcePack;
use mrpc_core::vehicle::VehicleResource;
use mrpc_core::script::VehicleScriptEntry;

fn big_pack(tag: &str, n: usize, rails: usize, files: usize) -> ResourcePack {
    let mut p = ResourcePack::new(&format!("pack {tag}"));
    for i in 0..n {
        p.resources.vehicles.push(VehicleResource {
            id: format!("{tag}_veh_{i}"),
            name: format!("vehicle {i} of {tag}"),
            script_id: Some(format!("{tag}_script_{i}")),
            ..Default::default()
        });
        p.resources.vehicle_scripts.push(VehicleScriptEntry {
            id: format!("{tag}_script_{i}"),
            ..Default::default()
        });
    }
    for i in 0..rails {
        p.resources.rails.push(serde_json::json!({
            "id": format!("{tag}_rail_{i}"),
            "name": format!("rail {i} of {tag}"),
            "model": "mtr:rail/default",
            "colour": "#FFFFFF",
        }));
    }
    for i in 0..files {
        p.set_resource(
            &format!("mtr:custom/{tag}/file_{i}.obj"),
            "obj",
            vec![0u8; 64],
        );
    }
    p
}

fn main() {
    // 400 vehicles/scripts each side, 3000 rails each side, 2000 files each
    // side — the shape of a realistic large-route pack merge.
    let mut a = big_pack("a", 400, 3000, 2000);
    let b = big_pack("b", 400, 3000, 2000);

    let t0 = std::time::Instant::now();
    let stats = a.merge_from(b);
    let dt = t0.elapsed();
    println!(
        "merged in {:?}: +{} vehicles, +{} scripts, +{} other, +{} files, {} renamed",
        dt,
        stats.vehicles_added,
        stats.scripts_added,
        stats.other_added,
        stats.files_added,
        stats.ids_renamed
    );
    assert_eq!(stats.vehicles_added, 400);
    assert_eq!(stats.other_added, 3000);
    assert_eq!(stats.files_added, 2000);

    // Same-pack-twice (full-dedup path): every incoming entry must be
    // recognised as already present.
    let a2 = big_pack("a", 400, 3000, 2000);
    let t0 = std::time::Instant::now();
    let stats2 = a2.clone().merge_from(a2);
    let dt2 = t0.elapsed();
    println!(
        "no-op re-merge in {:?}: added_anything={}",
        dt2,
        stats2.added_anything()
    );
    assert!(!stats2.added_anything());
}

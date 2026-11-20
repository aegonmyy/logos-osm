//! Live search verification against the real Nigeria PBF on this box.
//!
//! Not a unit test: it runs the full FFI op path (open -> import ->
//! search_build job -> geocode / reverse_geocode / snapshot) over the
//! 715 MB Geofabrik extract, asserting real answers. Run with:
//!
//!     cargo run -p logos-osm --example search_live --release
//!
//! (release: the debug-profile parser is ~10x slower over 135M nodes.)

use std::ffi::CString;
use std::time::Instant;

use serde_json::Value;

// The crate's own C-ABI entry points, called in-process (the same functions
// the Qt plugin resolves after dlopen; the example skips the dlopen).
use logos_osm::ffi::{logos_osm_free_string, logos_osm_invoke};

fn invoke(name: &str, args: Value) -> Value {
    let n = CString::new(name).unwrap();
    let a = CString::new(args.to_string()).unwrap();
    // SAFETY: both CStrings are valid and null-terminated; the returned
    // pointer is reclaimed via CString::from_raw below, matching the
    // into_raw the ABI used to produce it.
    let ptr = unsafe { logos_osm_invoke(n.as_ptr(), a.as_ptr()) };
    let s = unsafe { CString::from_raw(ptr) }
        .to_string_lossy()
        .to_string();
    unsafe { logos_osm_free_string(std::ptr::null_mut()) };
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

fn main() {
    let pbf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/codespace/osm-renderer-demo/nigeria-latest.osm.pbf".to_string());
    let out_dir = std::env::temp_dir().join(format!("osm-search-live-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let state = out_dir.join("state.json").display().to_string();
    let cache = out_dir.join("cache").display().to_string();

    // 1. open
    let r = invoke(
        "open",
        serde_json::json!({ "state_path": state, "cache_dir": cache, "memory": true }),
    );
    assert!(r["ok"].as_bool().unwrap(), "open failed: {r}");

    // 2. import the PBF (no network: local import validates framing)
    let t = Instant::now();
    let r = invoke(
        "import",
        serde_json::json!({ "region": "nigeria", "pbf": pbf, "index": false }),
    );
    assert!(r["ok"].as_bool().unwrap(), "import failed: {r}");
    println!(
        "import: {} s, {} blobs",
        t.elapsed().as_secs_f64(),
        r["result"]["blobs"]
    );

    // 3. search_build in JOB mode (the app's path), then poll job_status.
    let t = Instant::now();
    let r = invoke(
        "search_build",
        serde_json::json!({ "region": "nigeria", "job": true }),
    );
    assert!(
        r["ok"].as_bool().unwrap(),
        "search_build dispatch failed: {r}"
    );
    let job = r["result"]["job_id"].as_str().unwrap().to_string();
    println!("search_build job dispatched: {job}");
    loop {
        let s = invoke("job_status", serde_json::json!({ "job_id": job }));
        assert!(s["ok"].as_bool().unwrap(), "job_status failed: {s}");
        let st = s["result"]["state"].as_str().unwrap_or("running");
        if st == "running" {
            let el = s["result"]["elapsed"].as_u64().unwrap_or(0);
            print!("\r  indexing... {el} s   ");
            std::io::Write::flush(&mut std::io::stderr()).ok();
            std::thread::sleep(std::time::Duration::from_millis(500));
            continue;
        }
        println!("\nsearch_build {st}: {} s", t.elapsed().as_secs_f64());
        if st == "error" {
            panic!("search_build failed: {}", s["result"]["error"]);
        }
        let res = &s["result"]["result"];
        println!(
            "  places={} admins={} highways={} pois={} houses={} db={} bytes",
            res["places"],
            res["admin_areas"],
            res["highways"],
            res["pois"],
            res["houses"],
            res["bytes"]
        );
        break;
    }

    // 4. geocode: "Yaba, Lagos"
    for q in ["Yaba, Lagos", "Abuja", "Ibadan", "Kano"] {
        let r = invoke("geocode", serde_json::json!({ "query": q, "limit": 3 }));
        assert!(r["ok"].as_bool().unwrap(), "geocode failed: {r}");
        let results = r["result"]["results"].as_array().unwrap();
        println!("geocode {:?}: {} hit(s)", q, results.len());
        for h in results {
            println!(
                "  {} [{} {}] ({:.4}, {:.4}) admin={}",
                h["name"],
                h["class"],
                h["subclass"],
                h["lat"].as_f64().unwrap_or(0.0),
                h["lon"].as_f64().unwrap_or(0.0),
                h["admin_path"]
            );
        }
    }

    // 5. reverse_geocode: a Lagos point
    let r = invoke(
        "reverse_geocode",
        serde_json::json!({ "lat": 6.455, "lon": 3.407 }),
    );
    assert!(r["ok"].as_bool().unwrap(), "reverse_geocode failed: {r}");
    let hit = &r["result"];
    println!(
        "reverse (6.455, 3.407): {} [{} {}] {} m away, admin={}",
        hit["name"],
        hit["class"],
        hit["subclass"],
        hit["distance_m"].as_f64().map(|d| d.round()).unwrap_or(0.0),
        hit["admin_path"]
    );

    // 6. miss path: an unindexed region hint
    let r = invoke("geocode", serde_json::json!({ "query": "Fulda germany" }));
    assert!(r["ok"].as_bool().unwrap());
    let miss = &r["result"]["miss"];
    println!(
        "miss (\"Fulda germany\"): hint={} has_index={} msg={}",
        miss["hint"], miss["hint_has_index"], miss["message"]
    );

    // 7. index status
    let r = invoke("search_index_status", serde_json::json!({}));
    let idx = r["result"]["indexes"].as_array().unwrap();
    println!("status: {} index(es)", idx.len());
    for i in idx {
        println!(
            "  {} built={} places={} stale={}",
            i["region"], i["built"], i["places"], i["stale"]
        );
    }

    // 8. snapshot PNG for the Yaba hit
    let r = invoke(
        "geocode",
        serde_json::json!({ "query": "Yaba, Lagos", "limit": 1 }),
    );
    let hit = &r["result"]["results"][0];
    let lat = hit["lat"].as_f64().unwrap();
    let lon = hit["lon"].as_f64().unwrap();
    let r = invoke(
        "search_snapshot",
        serde_json::json!({ "region": "nigeria", "lat": lat, "lon": lon, "label": "Yaba", "admin_path": "Lagos" }),
    );
    assert!(r["ok"].as_bool().unwrap(), "snapshot failed: {r}");
    let b64 = r["result"]["png_base64"].as_str().unwrap();
    let png = base64_decode(b64);
    let out = out_dir.join("yaba.png");
    std::fs::write(&out, &png).unwrap();
    println!("snapshot: {} bytes -> {}", png.len(), out.display());
    assert_eq!(
        &png[0..8],
        &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
    );

    println!("LIVE VERIFICATION PASSED");
}

fn base64_decode(s: &str) -> Vec<u8> {
    let table = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            _ => 63,
        }
    };
    let mut out = Vec::new();
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            acc |= table(*b) << (18 - 6 * i);
        }
        out.push((acc >> 16) as u8);
        if chunk.len() > 1 {
            out.push((acc >> 8) as u8);
        }
        if chunk.len() > 2 {
            out.push(acc as u8);
        }
    }
    out
}

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Options;
use crate::engine::db::remote::Uri;
use crate::engine::db::{Database, object::Object};
use crate::engine::{AnyOutput, QueryKey};
use crate::query::image::{ImageObject, ImageSize};
use crate::query::js::FileOutput;
use crate::serde::SerializedMap;

// just for testing purposes, never refers to actual data.
fn o(n: u8) -> Object {
    unsafe { Object::from_hash([n; 32].into()) }
}

fn roundtrip<T: Serialize + Deserialize<'static> + 'static>(a1: &T) -> T {
    let bytes = postcard::to_stdvec(&a1).expect("serialization");
    // I really don't like this solution to satisfy Rust (we should have been able to just
    // deserialize on the stack given we never have references) but ah well leaking for testing is
    // probably fine I guess.
    let bytes = bytes.leak();
    let a2: T = postcard::from_bytes(bytes).expect("deserialization");
    a2
}

#[test]
fn roundtrip_result() {
    let a1 = Result::<(), ()>::Ok(());
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_obj() {
    let a1 = o(100);
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_any_output_obj() {
    let a1 = AnyOutput::new(crate::Result::Ok(o(100)));
    let a2 = roundtrip(&a1);
    assert_eq!(a1.0.type_id(), a2.0.type_id());
}

#[test]
fn roundtrip_file_output() {
    let mut a1: FileOutput = Default::default();
    a1.outputs = BTreeMap::from([(PathBuf::from("./index.html"), o(6))]);
    let a2 = roundtrip(&a1);
    assert_eq!(a1.outputs, a2.outputs);
}

#[test]
fn roundtrip_any_output_file_output() {
    let mut a1: FileOutput = Default::default();
    a1.outputs = BTreeMap::from([(PathBuf::from("./index.html"), o(7))]);
    let a1 = AnyOutput::new(crate::Result::Ok(a1));
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_vec_pathbuf() {
    let a1 = vec![PathBuf::from("./file.js")];
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_any_output_vec_pathbuf() {
    let a1 = vec![PathBuf::from("./file.js")];
    let a1 = AnyOutput::new(crate::Result::Ok(a1));
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_image_object() {
    let a1 = ImageObject {
        object: o(8),
        format: crate::query::image::ImageFormat::Jpeg,
        size: ImageSize {
            width: 69,
            height: 420,
        },
    };
    let a2 = roundtrip(&a1);
    assert_eq!(a1, a2);
}

#[test]
fn roundtrip_map() {
    let m1 = SerializedMap::<i32, i32>::default();
    let _ = m1.insert_sync(1, 2);
    let _ = m1.insert_sync(3, 4);
    let _ = m1.insert_sync(5, 6);

    let m2 = roundtrip(&m1);
    assert_eq!(m1.0, m2.0);
}

#[test]
fn roundtrip_database() {
    let db = Database::new(&Options::default());
    use crate::query::*;

    let k1 = QueryKey::GetUrl(GetUrl(Uri(hyper::Uri::from_static(
        "https://example.com/page1",
    ))));
    let k2 = QueryKey::ListDirectory(ListDirectory(PathBuf::from(".")));
    let k3 = QueryKey::MarkdownToHtml(MarkdownToHtml(o(3)));
    let k4 = QueryKey::MinifyHtml(MinifyHtml(o(4)));
    let k5 = QueryKey::ReadFile(ReadFile(PathBuf::from("./file.js")));
    let k6 = QueryKey::RunFile(RunFile {
        file: PathBuf::from("./file.js"),
        arg: None,
    });

    let db1 = futures_lite::future::block_on(async move {
        db.with_entry(k1.clone(), async |entry| {
            entry.insert(1, AnyOutput::new(crate::Result::Ok(o(1))));
        })
        .await;
        db.with_entry(k2.clone(), async |entry| {
            entry.insert(
                2,
                AnyOutput::new(crate::Result::Ok(vec![PathBuf::from("./file.js")])),
            );
        })
        .await;
        db.with_entry(k3.clone(), async |entry| {
            entry.insert(3, AnyOutput::new(crate::Result::Ok(o(3))));
        })
        .await;
        db.with_entry(k4.clone(), async |entry| {
            entry.insert(4, AnyOutput::new(crate::Result::Ok(o(4))));
        })
        .await;
        db.with_entry(k5.clone(), async |entry| {
            entry.insert(5, AnyOutput::new(crate::Result::Ok(o(5))));
        })
        .await;
        db.with_entry(k6.clone(), async |entry| {
            let mut output: FileOutput = Default::default();
            output.outputs = BTreeMap::from([(PathBuf::from("./index.html"), o(6))]);
            entry.insert(6, AnyOutput::new(crate::Result::Ok(output)));
        })
        .await;
        db.add_dependency(k1, k2).await;

        db
    });

    let _remotes2 = roundtrip(&db1.remotes);

    // This is more of a "can it serialize at all" test tbh, don't _really_ need to test
    // for equality right away.
    let _db2 = roundtrip(&db1.core);
}

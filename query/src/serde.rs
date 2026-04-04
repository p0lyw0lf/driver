use std::hash::Hash;
use std::marker::PhantomData;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;

/// Newtype for scc::HashMap that allows for serializing/deserializing, so long as the & is
/// actually an &mut or owned value.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct SerializedMap<K: Eq + Hash, V>(pub scc::HashMap<K, V>);

impl<K: Eq + Hash, V> Default for SerializedMap<K, V> {
    fn default() -> Self {
        Self(scc::HashMap::new())
    }
}

impl<K: Eq + Hash, V> Serialize for SerializedMap<K, V>
where
    K: Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Might panic in the presence of concurrent modifications...
        // Good thing we don't have those!!!
        let mut s = serializer.serialize_map(Some(self.len()))?;

        let mut entry = self.begin_sync();
        while let Some(e) = entry {
            s.serialize_entry(e.key(), e.get())?;
            entry = e.next_sync();
        }

        s.end()
    }
}

impl<'de, K: Eq + Hash, V> Deserialize<'de> for SerializedMap<K, V>
where
    K: Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<K: Eq + Hash, V>(PhantomData<(K, V)>);

        impl<'de, K: Eq + Hash, V> serde::de::Visitor<'de> for Visitor<K, V>
        where
            K: Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = SerializedMap<K, V>;

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let this = match map.size_hint() {
                    None => HashMap::new(),
                    Some(n) => HashMap::with_capacity(n),
                };

                let mut entry = map.next_entry()?;
                while let Some((k, v)) = entry {
                    let _ = this.insert_sync(k, v);
                    entry = map.next_entry()?;
                }

                Ok(SerializedMap(this))
            }

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "SerializedMap")
            }
        }

        deserializer.deserialize_map(Visitor(PhantomData))
    }
}

impl<K: Eq + Hash, V> std::ops::Deref for SerializedMap<K, V> {
    type Target = scc::HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K: Eq + Hash, V> std::ops::DerefMut for SerializedMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/*
mod test {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::SerializedMap;
    use super::SerializedMutex;
    use crate::db::Database;
    use crate::db::object::Object;
    use crate::js::FileOutput;
    use crate::js::RunFile;

    // just for testing purposes, never refers to actual data.
    fn obj(n: u8) -> Object {
        unsafe { Object::from_hash([n; 32].into()) }
    }

    #[test]
    fn any_output() {
        let a1 = AnyOutput::new(());

        let bytes = postcard::to_stdvec(&a1).expect("serialization");
        let a2: AnyOutput = postcard::from_bytes(&bytes[..]).expect("deserialization");
        assert_eq!(a1.0.type_id(), a2.0.type_id());
    }

    #[test]
    fn result() {
        let a1 = Result::<(), ()>::Ok(());

        let bytes = postcard::to_stdvec(&a1).expect("serialization");
        let a2: Result<(), ()> = postcard::from_bytes(&bytes[..]).expect("deserialization");
        assert_eq!(a1, a2);
    }

    #[test]
    fn any_output_result() {
        let a1 = AnyOutput::new(crate::Result::Ok(obj(100)));

        let bytes = postcard::to_stdvec(&a1).expect("serialization");
        let a2: AnyOutput = postcard::from_bytes(&bytes[..]).expect("deserialization");
        assert_eq!(a1.0.type_id(), a2.0.type_id());
    }

    #[test]
    fn roundtrip_map() {
        let m1 = SerializedMap::default();
        let _ = m1.insert_sync(1, 2);
        let _ = m1.insert_sync(3, 4);
        let _ = m1.insert_sync(5, 6);

        let bytes = postcard::to_stdvec(&m1).expect("serialization");
        let m2: SerializedMap<i32, i32> =
            postcard::from_bytes(&bytes[..]).expect("deserialization");
        assert_eq!(m1.0, m2.0);
    }

    #[test]
    fn roundtrip_mutex() {
        let v1 = SerializedMutex::new(123i32);

        let bytes = postcard::to_stdvec(&v1).expect("serialization");
        let v2: SerializedMutex<i32> = postcard::from_bytes(&bytes[..]).expect("deserialization");

        assert_eq!(*v1.0.lock().unwrap(), *v2.0.lock().unwrap());
    }

    #[test]
    fn roundtrip_database() {
        let db = Database::default();

        let k1 = QueryKey::GetUrl(GetUrl(
            url::Url::parse("https://example.com/page1").unwrap(),
        ));
        let k2 = QueryKey::ListDirectory(ListDirectory(PathBuf::from(".")));
        let k3 = QueryKey::MarkdownToHtml(MarkdownToHtml(obj(3)));
        let k4 = QueryKey::MinifyHtml(MinifyHtml(obj(4)));
        let k5 = QueryKey::ReadFile(ReadFile(PathBuf::from("./file.js")));
        let k6 = QueryKey::RunFile(RunFile {
            file: PathBuf::from("./file.js"),
            arg: Some(crate::js::JsValue::Store(crate::js::JsObject {
                object: obj(6),
            })),
        });

        let db1 = futures_lite::future::block_on(async move {
            db.with_entry(k1.clone(), async |mut entry| {
                entry.insert(1, AnyOutput::new(crate::Result::Ok(obj(1))));
            })
            .await;
            db.with_entry(k2.clone(), async |mut entry| {
                entry.insert(
                    2,
                    AnyOutput::new(crate::Result::Ok(vec![PathBuf::from("./file.js")])),
                );
            })
            .await;
            db.with_entry(k3.clone(), async |mut entry| {
                entry.insert(3, AnyOutput::new(crate::Result::Ok(obj(3))));
            })
            .await;
            db.with_entry(k4.clone(), async |mut entry| {
                entry.insert(4, AnyOutput::new(crate::Result::Ok(obj(4))));
            })
            .await;
            db.with_entry(k5.clone(), async |mut entry| {
                entry.insert(5, AnyOutput::new(crate::Result::Ok(obj(5))));
            })
            .await;
            db.with_entry(k6.clone(), async |mut entry| {
                entry.insert(
                    6,
                    AnyOutput::new(crate::Result::Ok(FileOutput {
                        value: crate::js::JsValue::Null,
                        outputs: BTreeMap::from([(PathBuf::from("./index.html"), obj(6))]),
                    })),
                );
            })
            .await;
            db.add_dependency(k1, k2).await;

            db
        });

        let bytes = postcard::to_stdvec(&db1).expect("serialization");
        let _db2: Database = postcard::from_bytes(&bytes[..]).expect("deserialization");

        // This is more of a "can it serialize at all" test tbh, don't _really_ need to test
        // for equality right away.
    }
}
*/

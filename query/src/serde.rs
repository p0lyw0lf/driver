use std::any::Any;
use std::any::TypeId;
use std::hash::Hash;
use std::marker::PhantomData;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;
use tokio::sync::Mutex;

use crate::query::context::AnyOutput;
use crate::query::context::Output;

/// Macro to help generate Serialization/Deserializationn for the AnyOutput type. It is very janky
/// I can't just use typeid because erased-serde isn't compatible with postcard.
macro_rules! valid_outputs {
    ($($ty:ty,)*) => {
$(
    impl Output for $ty {}
)*
    impl Output for AnyOutput {}

static INDEX_TO_TYPE_ID: &[TypeId] = &[$(
    TypeId::of::<$ty>(),
)*];

impl Serialize for AnyOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use std::ops::Deref;
        use serde::ser::{Error, SerializeTuple};

        // This is stupid but I have so few types the O(n)-ness doesn't matter
        let want = <dyn Any>::type_id(self.0.deref());
        let _i = INDEX_TO_TYPE_ID.iter().position(|t| {
            &want == t
        }).ok_or_else(|| S::Error::custom("type not found"))?;

        let mut s = serializer.serialize_tuple(2)?;
        s.serialize_element(&_i)?;
        $(
            if _i == 0 {
                let v = <dyn Any>::downcast_ref::<$ty>(self.0.deref()).expect("TypeId compared equal but couldn't downcast");
                s.serialize_element(v)?;
                return s.end();
            }
            let _i = _i.saturating_sub(1);
        )*
        unreachable!()
    }
}

impl<'de> Deserialize<'de> for AnyOutput
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{Visitor, SeqAccess, Error};

        struct TupleVisitor;
        impl<'de> Visitor<'de> for TupleVisitor {
            type Value = AnyOutput;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "AnyOutput")
            }

            #[inline]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>
            {
                let _i: usize = seq.next_element()?.ok_or_else(|| A::Error::custom("invalid length 0"))?;

                $(
                    if _i == 0 {
                        let v: $ty = seq.next_element()?.ok_or_else(|| A::Error::custom("invalid length 1"))?;
                        return Ok(AnyOutput::new(v));
                    }
                    let _i = _i.saturating_sub(1);
                )*
                Err(A::Error::custom("invalid tag"))
            }
        }

        deserializer.deserialize_tuple(2, TupleVisitor)
    }
}
    };
}

valid_outputs![
    crate::Result<crate::db::object::Object>,
    crate::Result<crate::js::FileOutput>,
    crate::Result<Vec<std::path::PathBuf>>,
    crate::Result<crate::query::image::ImageObject>,
    // Just for placeholder purposes, shouldn't show up in serialized DB
    (),
];

/// Newtype for scc::HashMap that allows for serializing/deserializing, so long as the & is
/// actually an &mut or owned value.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct SerializedMap<K: Eq + Hash, V>(pub HashMap<K, V>);

impl<K: Eq + Hash, V> Default for SerializedMap<K, V> {
    fn default() -> Self {
        Self(HashMap::new())
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
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K: Eq + Hash, V> std::ops::DerefMut for SerializedMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Newtype for tokio::Mutex that allow the things inside to be serialized.
#[derive(Debug, Default)]
pub struct SerializedMutex<T>(pub Mutex<T>);

impl<T> SerializedMutex<T> {
    pub fn new(t: T) -> Self {
        Self(Mutex::new(t))
    }
}

impl<T> Serialize for SerializedMutex<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.blocking_lock().serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for SerializedMutex<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self(Mutex::new(T::deserialize(deserializer)?)))
    }
}

impl<T> std::ops::Deref for SerializedMutex<T> {
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for SerializedMutex<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::SerializedMap;
    use super::SerializedMutex;
    use crate::db::Database;
    use crate::db::object::Object;
    use crate::js::FileOutput;
    use crate::js::RunFile;
    use crate::js::WriteOutput;
    use crate::query::context::AnyOutput;
    use crate::query::files::ListDirectory;
    use crate::query::files::ReadFile;
    use crate::query::html::MarkdownToHtml;
    use crate::query::html::MinifyHtml;
    use crate::query::key::QueryKey;
    use crate::query::remote::GetUrl;

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
    fn db_value() {
        let v1 = crate::db::Value {
            value: AnyOutput::new(crate::Result::Ok(obj(100))),
            color: crate::db::Color::Red,
            revision: 1,
        };

        let bytes = postcard::to_stdvec(&v1).expect("serialization");
        let v2: crate::db::Value = postcard::from_bytes(&bytes[..]).expect("deserialization");
        assert_eq!(v1.value.0.type_id(), v2.value.0.type_id());
        assert_eq!(v2.color, crate::db::Color::Green);
        assert_eq!(v2.revision, 0);
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

        assert_eq!(*v1.0.blocking_lock(), *v2.0.blocking_lock());
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let db1 = rt.block_on(async move {
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
                        outputs: vec![WriteOutput {
                            path: PathBuf::from("./index.html"),
                            object: obj(6),
                        }],
                    })),
                );
            })
            .await;
            db.add_dependency(k1, k2).await;

            db
        });

        let bytes = postcard::to_stdvec(&db1.dep_graph).expect("dep_graph serialization");
        let _dep_graph2: SerializedMap<QueryKey, std::collections::BTreeSet<QueryKey>> =
            postcard::from_bytes(&bytes[..]).expect("dep_graph deserialization");

        let bytes = postcard::to_stdvec(&db1.objects).expect("objects serialization");
        let _objects2: crate::db::object::Objects =
            postcard::from_bytes(&bytes[..]).expect("objects deserialization");

        let bytes = postcard::to_stdvec(&db1.remotes).expect("remotes serialization");
        let _remotes2: crate::db::remote::RemoteObjects =
            postcard::from_bytes(&bytes[..]).expect("remotes deserialization");

        let bytes = postcard::to_stdvec(&db1.cache).expect("cache serialization");
        let _cache2: SerializedMap<QueryKey, std::sync::Arc<SerializedMutex<crate::db::Value>>> =
            postcard::from_bytes(&bytes[..]).expect("cache deserialization");

        // This is more of a "can it serialize at all" test tbh, don't _really_ need to test these
        // immediately.
        // TODO: assert_eq!(db1.cache, db2.cache);
        // TODO: assert_eq!(db1.dep_graph, db2.dep_graph);
        assert_eq!(db1.objects, _objects2);
        // TODO: assert_eq!(db1.remotes, db2.remotes);
    }
}

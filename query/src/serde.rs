use std::hash::Hash;
use std::marker::PhantomData;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;
use tokio::sync::Mutex;

/// Newtype for scc::HashMap that allows for serializing/deserializing, so long as the & is
/// actually an &mut or owned value.
#[derive(Clone, Debug)]
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
    use super::SerializedMap;
    use super::SerializedMutex;
    use crate::db::Database;
    use crate::js::GetUrl;
    use crate::query::context::AnyOutput;
    use crate::query::key::QueryKey;

    #[test]
    fn roundtrip_any_output() {
        let a1 = AnyOutput::new(());

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

        assert_eq!(*v1.0.blocking_lock(), *v2.0.blocking_lock());
    }

    #[test]
    fn roundtrip_database() {
        let db = Database::default();
        let k1 = QueryKey::GetUrl(GetUrl(
            url::Url::parse("https://example.com/page1").unwrap(),
        ));
        let k2 = QueryKey::GetUrl(GetUrl(
            url::Url::parse("https://example.com/page2").unwrap(),
        ));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let db1 = rt.block_on(async move {
            db.with_entry(k1.clone(), async |mut entry| {
                entry.insert(1, AnyOutput::new(()));
            })
            .await;
            db.with_entry(k2.clone(), async |mut entry| {
                entry.insert(2, AnyOutput::new(()));
            })
            .await;
            db.add_dependency(k1, k2).await;
            db.as_serialized().await
        });

        let bytes = postcard::to_stdvec(&db1).expect("serialization");
        let db2 = postcard::from_bytes::<
            std::collections::HashMap<QueryKey, (AnyOutput, std::collections::BTreeSet<QueryKey>)>,
        >(&bytes[..])
        .expect("deserialization");
        assert_eq!(db1, db2);
    }
}

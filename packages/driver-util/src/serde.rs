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

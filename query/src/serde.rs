use std::marker::PhantomData;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serde::ser::SerializeMap;

/// Newtype for scc::HashMap that allows for serializing/deserializing, so long as the & is
/// actually an &mut or owned value.
#[derive(Clone, Debug)]
pub struct SerializedEntries<K: Eq + std::hash::Hash, V>(pub HashMap<K, V>);

impl<K: Eq + std::hash::Hash, V> Default for SerializedEntries<K, V> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<K: Eq + std::hash::Hash, V> Serialize for SerializedEntries<K, V>
where
    K: Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // TODO: If postcard doesn't support this, try doing Some(self.0.len()) instead.
        // Will be mighty unsafe in the presence of concurrent modifications however...
        let mut s = serializer.serialize_map(None)?;

        let mut entry = self.0.begin_sync();
        while let Some(e) = entry {
            s.serialize_entry(e.key(), e.get())?;
            entry = e.next_sync();
        }

        s.end()
    }
}

impl<'de, K: Eq + std::hash::Hash, V> Deserialize<'de> for SerializedEntries<K, V>
where
    K: Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct DeserializeVisitor<K: Eq + std::hash::Hash, V>(PhantomData<(K, V)>);

        impl<'de, K: Eq + std::hash::Hash, V> serde::de::Visitor<'de> for DeserializeVisitor<K, V>
        where
            K: Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = SerializedEntries<K, V>;

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let this = HashMap::new();

                let mut entry = map.next_entry()?;
                while let Some((k, v)) = entry {
                    let _ = this.insert_sync(k, v);
                    entry = map.next_entry()?;
                }

                Ok(SerializedEntries(this))
            }

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "expected map")
            }
        }

        deserializer.deserialize_map(DeserializeVisitor(PhantomData))
    }
}

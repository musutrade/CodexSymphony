//! Explicit missing-field defaults for JSON configuration. Values still pass
//! through the derived wire decoder: null, duplicate and unknown fields retain
//! their original validation. No intermediate JSON object discards duplicate keys.
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};

pub fn field<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
    name: &'static str,
    default: serde_json::Value,
) -> Result<T, D::Error> {
    deserializer.deserialize_any(DefaultVisitor {
        name,
        default,
        marker: PhantomData,
    })
}

struct DefaultVisitor<T> {
    name: &'static str,
    default: serde_json::Value,
    marker: PhantomData<T>,
}
impl<'de, T: Deserialize<'de>> Visitor<'de> for DefaultVisitor<T> {
    type Value = T;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a configuration object")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        T::deserialize(serde::de::value::MapAccessDeserializer::new(DefaultMap {
            map,
            name: self.name,
            default: self.default,
            seen: false,
            injecting: false,
        }))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, sequence: A) -> Result<T, A::Error> {
        T::deserialize(serde::de::value::SeqAccessDeserializer::new(sequence))
    }
}
struct DefaultMap<A> {
    map: A,
    name: &'static str,
    default: serde_json::Value,
    seen: bool,
    injecting: bool,
}
impl<'de, A: MapAccess<'de>> MapAccess<'de> for DefaultMap<A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        if let Some(key) = self.map.next_key::<String>()? {
            self.seen |= key == self.name;
            return seed.deserialize(key.into_deserializer()).map(Some);
        }
        if self.seen {
            return Ok(None);
        }
        self.seen = true;
        self.injecting = true;
        seed.deserialize(self.name.into_deserializer()).map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        if !self.injecting {
            return self.map.next_value_seed(seed);
        }
        self.injecting = false;
        let value = self.default.take();
        seed.deserialize(value).map_err(serde::de::Error::custom)
    }
}

use crate::error::{ApiResult, GoveeApiError};
use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlite_cache::{Cache, CacheConfig};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub static CACHE: Lazy<ArcSwap<Cache>> = Lazy::new(|| match open_cache() {
    Ok(cache) => cache.into(),
    Err(err) => panic!("{err}"),
});

fn cache_file_name() -> ApiResult<PathBuf> {
    let cache_dir = std::env::var("GOVEE2MQTT_CACHE_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs_next::cache_dir)
        .ok_or_else(|| {
            GoveeApiError::Config(
                "could not resolve a cache directory; set GOVEE2MQTT_CACHE_DIR \
                 to a writable path"
                    .into(),
            )
        })?;

    Ok(cache_dir.join("govee2mqtt-cache.sqlite"))
}

fn open_cache() -> ApiResult<Arc<Cache>> {
    let cache_file = cache_file_name()?;
    let conn = sqlite_cache::rusqlite::Connection::open(&cache_file).map_err(|err| {
        GoveeApiError::Io(format!("opening cache database {}: {err}", cache_file.display()).into())
    })?;
    let cache = Cache::new(
        // We have low cardinality and can be pretty relaxed
        CacheConfig {
            flush_gc_ratio: 1024,
            flush_interval: Duration::from_secs(900),
            max_ttl: None,
        },
        conn,
    )
    .map_err(|err| GoveeApiError::Io(format!("initialising cache: {err}").into()))?;
    Ok(Arc::new(cache))
}

pub fn purge_cache() -> ApiResult<()> {
    let cache_file = cache_file_name()?;
    std::fs::remove_file(&cache_file).map_err(|err| {
        GoveeApiError::Io(format!("removing cache file {}: {err}", cache_file.display()).into())
    })?;
    CACHE.store(open_cache()?);
    Ok(())
}

#[derive(Deserialize, Serialize, Debug)]
struct CacheEntry<T> {
    expires: DateTime<Utc>,
    result: CacheResult<T>,
}

#[derive(Deserialize, Serialize, Debug)]
enum CacheResult<T> {
    Ok(T),
    Err(String),
}

impl<T> CacheResult<T> {
    // Cached errors come back as their Display string from the original call.
    // We classify the rehydrated error as Api because the original cached
    // operation was always a remote call; the variant information is lost on
    // the way through serde, so this is the closest honest label.
    fn into_result(self) -> ApiResult<T> {
        match self {
            Self::Ok(v) => Ok(v),
            Self::Err(err) => Err(GoveeApiError::Api(err)),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CacheGetOptions<'a> {
    pub key: &'a str,
    pub topic: &'a str,
    pub soft_ttl: Duration,
    pub hard_ttl: Duration,
    pub negative_ttl: Duration,
    pub allow_stale: bool,
}

pub enum CacheComputeResult<T> {
    Value(T),
    WithTtl(T, Duration),
}

fn io<E: std::fmt::Display>(context: &str) -> impl FnOnce(E) -> GoveeApiError + '_ {
    move |err| GoveeApiError::Io(format!("{context}: {err}").into())
}

pub fn invalidate_key(topic: &str, key: &str) -> ApiResult<()> {
    let topic = CACHE
        .load()
        .topic(topic)
        .map_err(io("opening cache topic"))?;
    topic.delete(key).map_err(io("deleting cache key"))?;
    Ok(())
}

/// Cache an item with a soft TTL; we'll retry the operation
/// if the TTL has expired, but allow stale reads
pub async fn cache_get<T, Fut>(options: CacheGetOptions<'_>, future: Fut) -> ApiResult<T>
where
    T: Serialize + DeserializeOwned + std::fmt::Debug + Clone,
    Fut: Future<Output = ApiResult<CacheComputeResult<T>>>,
{
    let topic = CACHE
        .load()
        .topic(options.topic)
        .map_err(io("opening cache topic"))?;
    let (updater, current_value) = topic
        .get_for_update(options.key)
        .await
        .map_err(io("locking cache key for update"))?;
    let now = Utc::now();

    let mut cache_entry: Option<CacheEntry<T>> = None;

    if let Some(current) = &current_value {
        match serde_json::from_slice::<CacheEntry<T>>(&current.data) {
            Ok(entry) => {
                if now < entry.expires {
                    log::trace!("cache hit for {}", options.key);
                    return entry.result.into_result();
                }

                cache_entry.replace(entry);
            }
            Err(err) => {
                log::warn!(
                    "Error parsing CacheEntry: {err} {:?}",
                    String::from_utf8_lossy(&current.data)
                );
            }
        }
    }

    log::trace!("cache miss for {}", options.key);
    let value = future.await;
    match value {
        Ok(CacheComputeResult::WithTtl(value, ttl)) => {
            let entry = CacheEntry {
                expires: Utc::now() + ttl,
                result: CacheResult::Ok(value.clone()),
            };

            let data =
                serde_json::to_string_pretty(&entry).map_err(io("serialising cache entry"))?;
            updater
                .write(data.as_bytes(), options.hard_ttl)
                .map_err(io("writing cache entry"))?;
            Ok(value)
        }
        Ok(CacheComputeResult::Value(value)) => {
            let entry = CacheEntry {
                expires: Utc::now() + options.soft_ttl,
                result: CacheResult::Ok(value.clone()),
            };

            let data =
                serde_json::to_string_pretty(&entry).map_err(io("serialising cache entry"))?;
            updater
                .write(data.as_bytes(), options.hard_ttl)
                .map_err(io("writing cache entry"))?;
            Ok(value)
        }
        Err(err) => match cache_entry.take() {
            Some(mut entry) if options.allow_stale => {
                entry.expires = Utc::now() + options.negative_ttl;

                log::warn!("{err}, will use prior results");
                if matches!(&entry.result, CacheResult::Err(_)) {
                    entry.result = CacheResult::Err(format!("{err}"));
                }

                let data =
                    serde_json::to_string_pretty(&entry).map_err(io("serialising cache entry"))?;
                updater
                    .write(data.as_bytes(), options.hard_ttl)
                    .map_err(io("writing cache entry"))?;

                entry.result.into_result()
            }
            _ => {
                let entry = CacheEntry {
                    expires: Utc::now() + options.negative_ttl,
                    result: CacheResult::Err(format!("{err}")),
                };

                let data =
                    serde_json::to_string_pretty(&entry).map_err(io("serialising cache entry"))?;
                updater
                    .write(data.as_bytes(), options.hard_ttl)
                    .map_err(io("writing cache entry"))?;
                entry.result.into_result()
            }
        },
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchAddressBookEntry {
    pub id: String,
    pub address: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub source: String,
    pub enabled: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchAddressBookListResponse {
    pub entries: Vec<WatchAddressBookEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchAddressBookMutationResponse {
    pub status: String,
    pub entry: WatchAddressBookEntry,
}

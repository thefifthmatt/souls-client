use std::{collections::HashMap, sync::LazyLock};

// TODO read in properly
pub static ER_ENTITY_ID_TEMPLATES: LazyLock<HashMap::<u32, &str>> = LazyLock::new(|| HashMap::new());
pub static ER_GROUP_ENTITY_IDS: LazyLock<HashMap::<u32, u32>> = LazyLock::new(|| HashMap::new());
pub static DS1R_ENTITY_ID_TEMPLATES: LazyLock<HashMap::<u32, &str>> = LazyLock::new(|| HashMap::new());
pub static DS3_ENTITY_ID_TEMPLATES: LazyLock<HashMap::<u32, &str>> = LazyLock::new(|| HashMap::new());
pub static SDT_ENTITY_ID_TEMPLATES: LazyLock<HashMap::<u32, &str>> = LazyLock::new(|| HashMap::new());
 
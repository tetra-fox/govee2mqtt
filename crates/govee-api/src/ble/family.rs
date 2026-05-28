//! SKU-keyed family dispatch for the synthesized BLE-frame control surface.
//!
//! Each device family (projector, socket, ...) implements [`FamilyModule`]
//! and registers itself in [`FAMILIES`]. The flat [`entity_category`] /
//! [`entity_name`] / [`encode_capability`] / [`common_datas_seed`] helpers
//! iterate the registry so callers do not need to know which family owns a
//! given SKU or instance. This is what replaced the per-call `if sku !=
//! "H6093"` guards that earlier lived in each family module.
//!
//! State-mutation helpers that take family-specific structs
//! (`projector::apply_blob_field`, `projector::state_value`, etc.) stay as
//! direct calls into their family module; the trait owns the SKU-agnostic
//! surface, not the H6093-specific aurora-blob plumbing.
use crate::error::ApiResult;
use serde_json::Value as JsonValue;

/// One device family's synthesized-control surface. Methods return `None` when
/// the family does not own the supplied SKU or instance, so the registry can
/// try the next family or fall through.
pub trait FamilyModule: Send + Sync + 'static {
    /// SKUs this family owns. The registry checks this before calling the
    /// SKU-keyed methods so a family that does not handle the SKU is skipped
    /// without it having to repeat the check.
    fn supported_skus(&self) -> &'static [&'static str];

    /// HA `entity_category` for an instance. The outer `Option` is "is this
    /// instance mine?"; the inner `Option<String>` is the category value
    /// (`None` = HA Controls, `Some("config")` = HA Configuration).
    fn entity_category(&self, instance: &str) -> Option<Option<String>>;

    /// HA display name for an instance, or `None` if the family does not own
    /// it (callers then fall back to a generic humanizer).
    fn entity_name(&self, instance: &str) -> Option<&'static str>;

    /// Encode an instance command into base64 frames for `cmd:"ptReal"`.
    /// `None` means "not mine, try the next family or fall through".
    fn encode_capability(
        &self,
        sku: &str,
        instance: &str,
        value: &JsonValue,
    ) -> Option<ApiResult<Vec<String>>>;

    /// Seed source for held state, as `(bizType, bizKey)`. `None` if the
    /// family does not need common-datas seeding for this SKU.
    fn common_datas_seed(&self, sku: &str, device_id: &str) -> Option<(i32, String)>;
}

/// All registered family modules. Adding a family is one line here plus the
/// `impl FamilyModule` in its module.
static FAMILIES: &[&(dyn FamilyModule + Sync)] =
    &[&crate::ble::projector::Module, &crate::ble::socket::Module];

/// HA `entity_category` for an instance, across every registered family.
/// Returns the first owning family's value; `None` if no family owns it.
pub fn entity_category(instance: &str) -> Option<Option<String>> {
    FAMILIES.iter().find_map(|f| f.entity_category(instance))
}

/// HA display name for an instance, across every registered family.
pub fn entity_name(instance: &str) -> Option<&'static str> {
    FAMILIES.iter().find_map(|f| f.entity_name(instance))
}

/// Encode an instance command into base64 ptReal frames, across every
/// registered family. The registry pre-filters by `supported_skus` so a
/// family only sees SKUs it owns. `None` means no family handles this
/// `(sku, instance)` pair, so the caller falls back to the platform API.
pub fn encode_capability(
    sku: &str,
    instance: &str,
    value: &JsonValue,
) -> Option<ApiResult<Vec<String>>> {
    FAMILIES
        .iter()
        .filter(|f| f.supported_skus().contains(&sku))
        .find_map(|f| f.encode_capability(sku, instance, value))
}

/// Common-datas seed `(bizType, bizKey)` for a device, across every
/// registered family.
pub fn common_datas_seed(sku: &str, device_id: &str) -> Option<(i32, String)> {
    FAMILIES
        .iter()
        .filter(|f| f.supported_skus().contains(&sku))
        .find_map(|f| f.common_datas_seed(sku, device_id))
}

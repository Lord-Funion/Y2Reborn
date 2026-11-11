//! The codec rows' view of the radio daemon's codec decision (the policy itself runs in the platform's
//! `y2-radiod`, platform/crates/y2-radiod/src/bt/codec.rs). Negotiated PCM remains a separate
//! observation.
#![forbid(unsafe_code)]
use reborn_core::CodecPreference;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Capability {
    pub compiled_locally: bool,
    pub distribution_approved: bool,
    pub platform_qualified: bool,
    #[serde(default)]
    pub owner_private_experiment: bool,
}
#[derive(Clone, Default, Deserialize)]
pub struct Inventory {
    pub schema: u32,
    pub codecs: BTreeMap<String, Capability>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Eligibility {
    pub compiled_locally: bool,
    pub runtime_enabled: bool,
    // GetCodecs is the intersection, not a raw remote advertisement.
    pub mutually_usable: bool,
    pub remote_advertised: Option<bool>,
    pub distribution_approved: bool,
    pub platform_qualified: bool,
    pub auto_eligible: bool,
    pub experimental_eligible: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct PolicyContext {
    /// Measured load informs the next connection attempt only; never switch a
    /// playing transport simply because a scan or transient burst occurred.
    pub wifi_heavy_transfer: bool,
    pub cpu_pressure: bool,
    pub last_negotiated: Option<String>,
    pub repeated_failures: BTreeSet<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub context: PolicyContext,
    pub generation: u64,
    pub preference: CodecPreference,
    pub eligibility: BTreeMap<String, Eligibility>,
    pub visited: BTreeSet<String>,
    pub selected: Option<String>,
    pub requested: Option<String>,
    pub state: String,
    pub reason: Option<String>,
}
impl Session {
    pub fn new(
        generation: u64,
        preference: CodecPreference,
        inventory: &Inventory,
        runtime: &[String],
        mutual: &[String],
    ) -> Self {
        Self::new_experimental(generation, preference, inventory, runtime, mutual, false)
    }
    pub fn new_experimental(
        generation: u64,
        preference: CodecPreference,
        inventory: &Inventory,
        runtime: &[String],
        mutual: &[String],
        experimental: bool,
    ) -> Self {
        let eligibility = ["SBC", "AAC", "aptX", "aptX-HD", "LDAC"]
            .into_iter()
            .map(|name| {
                let cap = inventory.codecs.get(name).cloned().unwrap_or_default();
                let runtime_enabled = runtime.iter().any(|c| c == name);
                let mutually_usable = mutual.iter().any(|c| c == name);
                let auto_eligible = inventory.schema == 1
                    && cap.compiled_locally
                    && runtime_enabled
                    && mutually_usable
                    && cap.distribution_approved
                    && cap.platform_qualified;
                let experimental_eligible = experimental
                    && inventory.schema == 1
                    && cap.compiled_locally
                    && runtime_enabled
                    && mutually_usable
                    && (cap.distribution_approved || cap.owner_private_experiment);
                (
                    name.into(),
                    Eligibility {
                        compiled_locally: cap.compiled_locally,
                        runtime_enabled,
                        mutually_usable,
                        remote_advertised: None,
                        distribution_approved: cap.distribution_approved,
                        platform_qualified: cap.platform_qualified,
                        auto_eligible,
                        experimental_eligible,
                    },
                )
            })
            .collect();
        Self {
            context: PolicyContext::default(),
            generation,
            preference,
            eligibility,
            visited: BTreeSet::new(),
            selected: None,
            requested: None,
            state: "Ready".into(),
            reason: None,
        }
    }
    /// The radio daemon's codec decision for the connected headset, as the codec rows read it.
    pub fn from_daemon(codec: &y2p_contract::radiod::Codec) -> Self {
        let preference =
            serde_json::from_value(serde_json::Value::String(codec.preference.clone()))
                .unwrap_or_default();
        Self {
            context: PolicyContext::default(),
            generation: codec.generation,
            preference,
            eligibility: codec
                .eligibility
                .iter()
                .map(|(name, e)| {
                    (
                        name.clone(),
                        Eligibility {
                            compiled_locally: e.compiled_locally,
                            runtime_enabled: e.runtime_enabled,
                            mutually_usable: e.mutually_usable,
                            remote_advertised: None,
                            distribution_approved: e.distribution_approved,
                            platform_qualified: e.platform_qualified,
                            auto_eligible: e.auto_eligible,
                            experimental_eligible: e.experimental_eligible,
                        },
                    )
                })
                .collect(),
            visited: BTreeSet::new(),
            selected: None,
            requested: None,
            state: codec.state.clone(),
            reason: codec.reason.clone(),
        }
    }
}

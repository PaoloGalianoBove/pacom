use up_rust::UUri;

pub fn is_mqtt_wildcard_ue_id(ue_id: u32) -> bool {
    ue_id == 0xFFFF || ue_id == u32::MAX
}

pub fn is_wildcard_resource_id(resource_id: u32) -> bool {
    resource_id == 0xFFFF || resource_id == u32::MAX
}

pub fn normalize_uri_for_vsomeip(uri: &UUri) -> UUri {
    UUri::try_from_parts(
        "",
        uri.ue_id,
        uri.ue_version_major as u8,
        uri.resource_id as u16,
    )
    .unwrap_or_else(|_| uri.clone())
}

/// Resolves and expands uProtocol UUri filters into discrete vSomeIP candidates.
pub struct VsomeipTopologyResolver {
    authority: String,
}

impl VsomeipTopologyResolver {
    pub fn new(authority: String) -> Self {
        Self { authority }
    }

    /// Returns concrete local vSomeIP listener filters without using wildcard major matching.
    pub fn local_listener_candidates(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
    ) -> Vec<UUri> {
        let source_auth = source_filter.authority_name();
        let source_uses_any_authority = source_auth.is_empty() || source_auth == "*";
        let source_uses_any_ue = is_mqtt_wildcard_ue_id(source_filter.ue_id) || source_filter.ue_id == 0;

        if let Some(sink) = sink_filter {
            if source_uses_any_ue && is_wildcard_resource_id(sink.resource_id) {
                return vec![];
            }

            if source_uses_any_authority && source_uses_any_ue {
                let resource = if is_wildcard_resource_id(source_filter.resource_id) {
                    sink.resource_id as u16
                } else {
                    source_filter.resource_id as u16
                };

                if let Ok(uri) = UUri::try_from_parts(
                    "",
                    sink.ue_id,
                    sink.ue_version_major as u8,
                    resource,
                ) {
                    return vec![uri];
                }
            }
        }

        let entity_id = source_filter.ue_id;
        let version_major = source_filter.ue_version_major as u8;
        let resource_id = source_filter.resource_id as u16;

        let normalized_source = UUri::try_from_parts("", entity_id, version_major, resource_id)
            .unwrap_or_else(|_| source_filter.clone());
        let local_authority_source = UUri::try_from_parts(
            &self.authority,
            entity_id,
            version_major,
            resource_id,
        )
        .unwrap_or_else(|_| normalized_source.clone());

        let candidates = if !source_uses_any_authority && source_auth != self.authority {
            vec![normalized_source.clone(), local_authority_source, source_filter.clone()]
        } else if source_uses_any_authority {
            vec![normalized_source.clone(), local_authority_source]
        } else {
            vec![source_filter.clone(), normalized_source.clone()]
        };

        let mut unique = std::collections::HashSet::new();
        candidates
            .into_iter()
            .filter(|candidate| unique.insert(candidate.to_uri(false)))
            .collect()
    }
}

use up_rust::UUri;

pub fn is_mqtt_wildcard_ue_id(ue_id: u32) -> bool {
    ue_id == 0xFFFF || ue_id == u32::MAX
}

pub fn is_wildcard_resource_id(resource_id: u32) -> bool {
    resource_id == 0xFFFF || resource_id == u32::MAX
}

pub fn is_wildcard_major_version(major: u32) -> bool {
    major == 0xFF
}

pub fn is_wildcard_source_filter(uri: &UUri) -> bool {
    let auth = uri.authority_name();
    (auth.is_empty() || auth == "*") && (is_mqtt_wildcard_ue_id(uri.ue_id) || uri.ue_id == 0)
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

    /// Normalizes a wildcard source filter when it explicitly targets a local sink.
    pub fn normalized_local_source_filter(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        is_cloud_bound_sink: bool,
    ) -> UUri {
        let Some(sink) = sink_filter else {
            return source_filter.clone();
        };

        if is_wildcard_source_filter(source_filter) && !is_cloud_bound_sink {
            let resource = if is_wildcard_resource_id(source_filter.resource_id) {
                sink.resource_id as u16
            } else {
                source_filter.resource_id as u16
            };

            if let Ok(uri) =
                UUri::try_from_parts("", sink.ue_id, sink.ue_version_major as u8, resource)
            {
                return uri;
            }
        }

        source_filter.clone()
    }

    pub fn should_skip_local_vsomeip_catchall(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        is_cloud_bound_sink: bool,
    ) -> bool {
        let Some(sink) = sink_filter else {
            return false;
        };

        is_wildcard_source_filter(source_filter)
            && !is_cloud_bound_sink
            && is_wildcard_resource_id(sink.resource_id)
    }

    pub fn should_skip_local_vsomeip_candidate(
        &self,
        candidate: &UUri,
        sink_filter: Option<&UUri>,
        is_cloud_bound_sink: bool,
    ) -> bool {
        let Some(_sink) = sink_filter else {
            return false;
        };

        !is_cloud_bound_sink
            && (is_mqtt_wildcard_ue_id(candidate.ue_id)
                || is_wildcard_major_version(candidate.ue_version_major)
                || is_wildcard_resource_id(candidate.resource_id))
    }

    /// Expands a given source filter and sink filter into a deduplicated list
    /// of valid vSomeIP UUri candidates, skipping invalid or conflicting ones.
    pub fn expand_listener_candidates(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        is_cloud_bound_sink: bool,
    ) -> Vec<UUri> {
        if self.should_skip_local_vsomeip_catchall(source_filter, sink_filter, is_cloud_bound_sink) {
            return vec![];
        }

        let normalized_source = self.normalized_local_source_filter(source_filter, sink_filter, is_cloud_bound_sink);
        let entity_id = normalized_source.ue_id;
        let version_major = normalized_source.ue_version_major as u8;
        let resource_id = normalized_source.resource_id;

        let filter_local = UUri::try_from_parts(
            &self.authority,
            entity_id,
            version_major,
            resource_id as u16,
        )
        .unwrap_or_else(|_| normalized_source.clone());
        
        let filter_empty =
            UUri::try_from_parts("", entity_id, version_major, resource_id as u16)
                .unwrap_or_else(|_| normalized_source.clone());

        let source_auth = normalized_source.authority_name();
        let mut candidates: Vec<UUri> = if !source_auth.is_empty()
            && source_auth != "*"
            && source_auth != self.authority
        {
            vec![
                filter_empty.clone(),
                filter_local.clone(),
                normalized_source.clone(),
            ]
        } else {
            vec![
                normalized_source.clone(),
                filter_local.clone(),
                filter_empty.clone(),
            ]
        };

        candidates.dedup_by(|a, b| a.to_uri(false) == b.to_uri(false));

        candidates
            .into_iter()
            .filter(|candidate| {
                !self.should_skip_local_vsomeip_candidate(candidate, sink_filter, is_cloud_bound_sink)
            })
            .collect()
    }
}

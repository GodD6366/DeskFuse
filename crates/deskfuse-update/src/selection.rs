use crate::{ArtifactDescriptor, ReleaseVersion, Target, UpdateError, VerifiedRelease};
use deskfuse_core::{Capabilities, ConnectionContext};
use semver::Version;
use std::{cmp::Ordering, collections::BTreeSet};
use uuid::Uuid;

/// Inspect fresh session evidence on both sender and receiver, including after rerouting.
#[must_use]
pub fn permits_lan_update(
    paired: bool,
    negotiated: Capabilities,
    context: &ConnectionContext,
) -> bool {
    paired && negotiated.contains(Capabilities::LAN_UPDATES) && context.input_eligible()
}

pub struct Installation<'a> {
    pub release: &'a ReleaseVersion,
    pub target: Target,
    pub os: &'a Version,
    pub protocol_major: u32,
    pub update_protocol: u32,
    pub quarantined: &'a BTreeSet<Uuid>,
}

/// Only locally verified manifests may enter selection. Transport has already
/// filtered inventories to paired, live physical-LAN sessions.
///
/// # Errors
/// Returns a conflict if eligible equal-version manifests disagree.
pub fn select_release<'a>(
    installed: &Installation<'_>,
    offers: &'a [VerifiedRelease],
) -> Result<Option<(&'a VerifiedRelease, &'a ArtifactDescriptor)>, UpdateError> {
    let mut candidates = Vec::new();
    for offer in offers {
        let manifest = offer.manifest();
        let Some(artifact) = offer.artifact(installed.target) else {
            continue;
        };
        if manifest.protocol_major != installed.protocol_major
            || manifest.minimum_update_protocol > installed.update_protocol
            || artifact.minimum_os > *installed.os
            || manifest.release.precedence(installed.release) != Ordering::Greater
        {
            continue;
        }
        candidates.push((offer, artifact));
    }
    // Fail closed for any eligible equal-version conflict, independent of arrival order.
    for (i, (left, _)) in candidates.iter().enumerate() {
        for (right, _) in candidates.iter().skip(i + 1) {
            if left
                .manifest()
                .release
                .precedence(&right.manifest().release)
                == Ordering::Equal
                && left.manifest() != right.manifest()
            {
                return Err(UpdateError::Conflict);
            }
        }
    }
    Ok(candidates
        .into_iter()
        .filter(|(release, _)| {
            !installed
                .quarantined
                .contains(&release.manifest().release.release_id)
        })
        .max_by(|(a, _), (b, _)| a.manifest().release.precedence(&b.manifest().release)))
}

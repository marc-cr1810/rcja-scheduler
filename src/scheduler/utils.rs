use crate::model::{Activity, Field, FieldKind, Volunteer};

pub fn is_field_suitable_for_activity(
    config: &crate::model::TournamentConfig,
    field: &Field,
    activity: &Activity,
) -> bool {
    let div_id = activity.division_id();

    match activity {
        Activity::Interview { .. } => {
            if field.kind != FieldKind::Interview {
                return false;
            }
        }
        _ => {
            if field.kind != FieldKind::Competition {
                return false;
            }
        }
    }

    // Check field-level restrictions (which divisions are allowed on this field)
    if let Some(ref allowed_divs) = field.allowed_divisions
        && !allowed_divs.contains(&div_id.to_string())
    {
        return false;
    }

    // Check division-level restrictions (which fields this division is allowed on)
    if let Some(div) = config.divisions.iter().find(|d| d.id == div_id)
        && let Some(ref allowed_fields) = div.allowed_fields
        && !allowed_fields.contains(&field.id)
    {
        return false;
    }

    true
}

pub fn is_volunteer_qualified(volunteer: &Volunteer, activity: &Activity, div_id: &str) -> bool {
    if let Some(ref capabilities) = volunteer.capabilities {
        if matches!(activity, Activity::Interview { .. }) {
            capabilities.contains(&"Interview".to_string())
                || capabilities.contains(&div_id.to_string())
        } else {
            capabilities.contains(&div_id.to_string())
        }
    } else {
        true
    }
}

pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Returns an id based on `base` that does not collide with any string in
/// `existing`, appending `_2`, `_3`, … if needed. Because [`sanitize_name`]
/// strips all separators, distinct display names like "U12" and "U 12" sanitize
/// to the same base; this keeps their ids distinct so teams don't silently land
/// in the wrong division/field.
pub fn unique_id(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{}_{}", base, n);
        if !existing.iter().any(|e| *e == candidate) {
            return candidate;
        }
        n += 1;
    }
}

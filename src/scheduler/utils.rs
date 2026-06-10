use crate::model::{Activity, Field, FieldKind, Volunteer};

pub fn is_field_suitable_for_activity(config: &crate::model::TournamentConfig, field: &Field, activity: &Activity) -> bool {
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
        && !allowed_divs.contains(&div_id.to_string()) {
            return false;
        }

    // Check division-level restrictions (which fields this division is allowed on)
    if let Some(div) = config.divisions.iter().find(|d| d.id == div_id)
        && let Some(ref allowed_fields) = div.allowed_fields
            && !allowed_fields.contains(&field.id) {
                return false;
            }

    true
}

pub fn is_volunteer_qualified(volunteer: &Volunteer, activity: &Activity, div_id: &str) -> bool {
    if let Some(ref capabilities) = volunteer.capabilities {
        if matches!(activity, Activity::Interview { .. }) {
            capabilities.contains(&"Interview".to_string()) || capabilities.contains(&div_id.to_string())
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

pub fn format_minutes_to_time(min: u32) -> String {
    let h = min / 60;
    let m = min % 60;
    format!("{:02}:{:02}", h, m)
}

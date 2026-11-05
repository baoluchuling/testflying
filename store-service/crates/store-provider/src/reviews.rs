//! Public review filtering is page-local, preserving the upstream continuation cursor.
use super::*;
use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
pub(super) struct Filters {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    timezone: String,
    locale: String,
    territory: String,
    rating: Option<i64>,
}
impl Filters {
    pub(super) fn parse(p: &Value) -> Result<Self> {
        let timezone = if s(p, "timezone").trim().is_empty() {
            "Asia/Shanghai"
        } else {
            s(p, "timezone").trim()
        };
        let zone: chrono_tz::Tz = timezone.parse().map_err(|_| {
            StoreError::new(
                422,
                "invalid_timezone",
                "timezone must be an IANA time zone",
            )
        })?;
        let selected = s(p, "date").trim();
        let start = s(p, "startDate").trim();
        let end = s(p, "endDate").trim();
        if !selected.is_empty() && (!start.is_empty() || !end.is_empty()) {
            return Err(date_error("date cannot be combined with startDate/endDate"));
        }
        let parse = |raw: &str, end: bool| -> Result<Option<DateTime<Utc>>> {
            if raw.is_empty() {
                return Ok(None);
            }
            let day = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|_| date_error("Review dates must use YYYY-MM-DD"))?;
            if day.format("%Y-%m-%d").to_string() != raw {
                return Err(date_error("Review dates must use YYYY-MM-DD"));
            }
            let day = if end {
                day.checked_add_days(Days::new(1))
                    .ok_or_else(|| date_error("Review date is out of range"))?
            } else {
                day
            };
            let midnight = day
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| date_error("Invalid review date"))?;
            let value = zone
                .from_local_datetime(&midnight)
                .earliest()
                .ok_or_else(|| date_error("Review date does not exist in this time zone"))?
                .with_timezone(&Utc);
            Ok(Some(value))
        };
        let start = parse(if selected.is_empty() { start } else { selected }, false)?;
        let end = parse(if selected.is_empty() { end } else { selected }, true)?;
        if start.zip(end).is_some_and(|(s, e)| s >= e) {
            return Err(date_error("startDate must not be after endDate"));
        }
        let rating = match p.get("rating") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_i64()
                    .filter(|n| (1..=5).contains(n))
                    .ok_or_else(|| invalid("rating must be 1–5"))?,
            ),
        };
        Ok(Self {
            start,
            end,
            timezone: timezone.into(),
            locale: s(p, "locale").trim().into(),
            territory: s(p, "territory").trim().into(),
            rating,
        })
    }
    pub(super) fn apply(&self, response: &mut Value) {
        if let Some(reviews) = response["reviews"].as_array_mut() {
            reviews.retain(|r| {
                if self.rating.is_some_and(|n| r["rating"].as_i64() != Some(n)) {
                    return false;
                }
                if !self.locale.is_empty() && !s(r, "locale").eq_ignore_ascii_case(&self.locale) {
                    return false;
                }
                if !self.territory.is_empty()
                    && !s(r, "territory").eq_ignore_ascii_case(&self.territory)
                {
                    return false;
                }
                if self.start.is_some() || self.end.is_some() {
                    let raw = if s(r, "createdAt").is_empty() {
                        s(r, "updatedAt")
                    } else {
                        s(r, "createdAt")
                    };
                    let Ok(date) = DateTime::parse_from_rfc3339(raw) else {
                        return false;
                    };
                    let date = date.with_timezone(&Utc);
                    if self.start.is_some_and(|start| date < start)
                        || self.end.is_some_and(|end| date >= end)
                    {
                        return false;
                    }
                }
                true
            });
        }
        response["filters"] = json!({"startAt":self.start.map(|d|d.to_rfc3339()).unwrap_or_default(),
            "endAt":self.end.map(|d|(d-chrono::Duration::microseconds(1)).to_rfc3339()).unwrap_or_default(),
            "timezone":self.timezone,"locale":self.locale,"territory":self.territory,"rating":self.rating});
    }
}
fn date_error(message: &str) -> StoreError {
    StoreError::new(422, "invalid_review_date_filter", message)
}

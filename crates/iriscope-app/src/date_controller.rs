//! Localized calendar shared by library and comparison filters.
use crate::ui::{AppState, CalendarDayData, MainWindow};
use chrono::{Datelike, Months, NaiveDate};
use slint::{ComponentHandle, ModelRc, VecModel};
pub(super) fn parse(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%d/%m/%Y")
        .or_else(|_| NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d"))
        .ok()
}
fn show_month(win: &MainWindow, day: NaiveDate) {
    let first = day.with_day(1).unwrap();
    let offset = first.weekday().num_days_from_monday();
    let month_names = [
        "Janvier",
        "Février",
        "Mars",
        "Avril",
        "Mai",
        "Juin",
        "Juillet",
        "Août",
        "Septembre",
        "Octobre",
        "Novembre",
        "Décembre",
    ];
    let s = win.global::<AppState>();
    s.set_calendar_month(first.format("%Y-%m-%d").to_string().into());
    s.set_calendar_caption(
        format!(
            "{} {}",
            month_names[usize::try_from(first.month0()).unwrap()],
            first.year()
        )
        .into(),
    );
    s.set_calendar_days(ModelRc::new(VecModel::from(
        (0..42)
            .map(|i| {
                let date = first + chrono::Duration::days(i - i64::from(offset));
                CalendarDayData {
                    day: i32::try_from(date.day()).unwrap(),
                    date: date.format("%d/%m/%Y").to_string().into(),
                    enabled: date.month() == first.month(),
                    selected: parse(&s.get_calendar_selection()) == Some(date),
                }
            })
            .collect::<Vec<_>>(),
    )));
}
pub(super) fn install(window: &MainWindow) {
    let weak = window.as_weak();
    window.global::<AppState>().on_open_calendar(move |target| {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let s = win.global::<AppState>();
        let value = match target {
            1 => s.get_library_query_from(),
            2 => s.get_library_query_to(),
            3 => s.get_comparison_from(),
            _ => s.get_comparison_to(),
        };
        s.set_calendar_target(target);
        s.set_calendar_open(true);
        let day = parse(&value).unwrap_or_else(|| chrono::Local::now().date_naive());
        s.set_calendar_selection(day.format("%Y-%m-%d").to_string().into());
        show_month(&win, day);
    });
    let weak = window.as_weak();
    window.global::<AppState>().on_calendar_step(move |offset| {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let s = win.global::<AppState>();
        let Some(day) = parse(&s.get_calendar_month()) else {
            return;
        };
        let next = if offset < 0 {
            day.checked_sub_months(Months::new(1))
        } else {
            day.checked_add_months(Months::new(1))
        };
        if let Some(next) = next {
            show_month(&win, next);
        }
    });
    let weak = window.as_weak();
    window
        .global::<AppState>()
        .on_select_calendar_date(move |value| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if !value.is_empty() && parse(&value).is_none() {
                return;
            }
            match s.get_calendar_target() {
                1 => s.set_library_query_from(value),
                2 => s.set_library_query_to(value),
                3 => s.set_comparison_from(value),
                _ => s.set_comparison_to(value),
            }
            s.set_calendar_open(false);
        });
}

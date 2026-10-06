//! Width layout for the human tables: which columns a width allows, and how
//! far the two shrinkable columns are cut to fit.

use super::types::{shorten_end, shorten_location};
use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

/// A column is never narrower than its header, and the shrinkable headers
/// ("Function", "Location") are 8 columns wide.
pub(crate) const HEADER_FLOOR: usize = 8;

/// The narrowest a Function cell is cut to: its header's width.
pub(crate) const FUNCTION_FLOOR: usize = HEADER_FLOOR;

pub(crate) const FUNCTION_HEADER: &str = "Function";
pub(crate) const LOCATION_HEADER: &str = "Location";

/// What a width allows: the coverage bar's cells and the CC column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tier {
    pub bar: usize,
    pub cc: bool,
}

/// The columns a width allows. No limit is the full layout.
pub(crate) fn tier(width: Option<usize>) -> Tier {
    let width = width.unwrap_or(usize::MAX);
    let bar = match width {
        100.. => 10,
        80..=99 => 5,
        _ => 0,
    };
    Tier {
        bar,
        cc: width >= 60,
    }
}

/// The width of a table whose columns hold `content` columns of text each:
/// a border and a space either side of every cell, plus the closing border.
pub(crate) fn table_width(content: &[usize]) -> usize {
    content.iter().sum::<usize>() + 3 * content.len() + 1
}

/// How a shrinkable column is cut.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Cut {
    /// Drop the end, as for a function name.
    End,
    /// Drop the start at a path separator, keeping `<file>:<line>`.
    Location,
}

impl Cut {
    /// `text` cut to `budget`, or whole when there is no budget.
    pub(crate) fn apply(
        self,
        text: &str,
        budget: Option<usize>,
    ) -> Cow<'_, str> {
        match (self, budget) {
            (_, None) => Cow::Borrowed(text),
            (Self::End, Some(budget)) => shorten_end(text, budget),
            (Self::Location, Some(budget)) => shorten_location(text, budget),
        }
    }
}

/// The width of a column headed `header` over `values`, each cut to `budget`.
pub(crate) fn column_width(
    header: &str,
    values: &[&str],
    budget: Option<usize>,
    cut: Cut,
) -> usize {
    values
        .iter()
        .map(|value| cut.apply(value, budget).width())
        .fold(header.width(), usize::max)
}

/// Whether Function and Location, cut to their floors, fit `room`.
pub(crate) fn can_fit(
    room: usize,
    functions: &[&str],
    locations: &[&str],
) -> bool {
    column_width(FUNCTION_HEADER, functions, Some(0), Cut::End)
        + column_width(LOCATION_HEADER, locations, Some(0), Cut::Location)
        <= room
}

/// How far the Function and Location columns are cut. `None` leaves a
/// column whole.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budgets {
    pub function: Option<usize>,
    pub location: Option<usize>,
}

/// Fit the Function and Location columns into `room` columns of text.
/// Location gives way first, down to `…/<file>:<line>`, then Function,
/// down to its header. When even those floors do not fit, both stay at
/// their floors and the table is wider than `room` allows.
pub(crate) fn fit(
    room: usize,
    functions: &[&str],
    locations: &[&str],
) -> Budgets {
    let function_whole = column_width(FUNCTION_HEADER, functions, None, Cut::End);
    let location_whole = column_width(LOCATION_HEADER, locations, None, Cut::Location);
    if function_whole + location_whole <= room {
        return Budgets::default();
    }
    let location_floor = column_width(LOCATION_HEADER, locations, Some(0), Cut::Location);
    let location = room.saturating_sub(function_whole).max(location_floor);
    let location_used = column_width(LOCATION_HEADER, locations, Some(location), Cut::Location);
    let function = (function_whole + location_used > room)
        .then(|| room.saturating_sub(location_used).max(FUNCTION_FLOOR));
    Budgets {
        function,
        location: Some(location),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_width_decides_the_bar_and_the_cc_column() {
        assert_eq!(tier(None), Tier { bar: 10, cc: true });
        assert_eq!(tier(Some(100)), Tier { bar: 10, cc: true });
        assert_eq!(tier(Some(99)), Tier { bar: 5, cc: true });
        assert_eq!(tier(Some(80)), Tier { bar: 5, cc: true });
        assert_eq!(tier(Some(79)), Tier { bar: 0, cc: true });
        assert_eq!(tier(Some(60)), Tier { bar: 0, cc: true });
        assert_eq!(tier(Some(59)), Tier { bar: 0, cc: false });
        assert_eq!(tier(Some(1)), Tier { bar: 0, cc: false });
    }

    #[test]
    fn a_table_is_its_content_plus_padding_and_borders() {
        // "│ a ┆ bb │" is 1 + 3 + 1 + 4 + 1 columns.
        assert_eq!(table_width(&[1, 2]), 10);
        assert_eq!(table_width(&[]), 1);
    }

    #[test]
    fn content_that_fits_is_not_cut() {
        let budgets = fit(40, &["run"], &["src/main.rs:12"]);
        assert_eq!(budgets, Budgets::default());
    }

    #[test]
    fn location_is_cut_before_function() {
        let budgets = fit(
            40,
            &["long_function_name"],
            &["src/report/pr_comment.rs:380"],
        );
        assert_eq!(budgets.function, None);
        assert_eq!(budgets.location, Some(22));
    }

    #[test]
    fn function_is_cut_once_location_is_at_its_file_and_line() {
        let budgets = fit(
            30,
            &["write_pr_comment_hot_spots"],
            &["src/report/pr_comment.rs:380"],
        );
        // "…/pr_comment.rs:380" is 19 columns, which leaves 11 for Function.
        assert_eq!(budgets.location, Some(19));
        assert_eq!(budgets.function, Some(11));
    }

    #[test]
    fn function_stays_whole_when_the_cut_location_fits_exactly() {
        // Function 8 + "…/pr_comment.rs:380" 19 is exactly the room.
        let budgets = fit(27, &["run"], &["src/report/pr_comment.rs:380"]);
        assert_eq!(budgets.location, Some(19));
        assert_eq!(budgets.function, None);
    }

    #[test]
    fn neither_column_goes_below_its_header() {
        let budgets = fit(4, &["write_pr_comment_hot_spots"], &["lib.rs:3"]);
        assert_eq!(budgets.function, Some(FUNCTION_FLOOR));
        assert_eq!(budgets.location, Some(HEADER_FLOOR));
    }

    #[test]
    fn can_fit_compares_the_floors_with_the_room() {
        let functions = ["write_pr_comment_hot_spots"];
        let locations = ["src/report/pr_comment.rs:380"];
        // Floors: Function 8, Location "…/pr_comment.rs:380" 19.
        assert!(can_fit(27, &functions, &locations));
        assert!(!can_fit(26, &functions, &locations));
    }

    proptest! {
        /// Narrowing the width never brings back a column or bar cells it
        /// has dropped.
        #[test]
        fn narrowing_never_brings_a_column_back(a in 0usize..200, b in 0usize..200) {
            let (wide, narrow) = (a.max(b), a.min(b));
            let (wide, narrow) = (tier(Some(wide)), tier(Some(narrow)));
            prop_assert!(narrow.bar <= wide.bar);
            prop_assert!(!narrow.cc || wide.cc);
        }

        /// The cut columns fit `room` whenever their floors do, and a
        /// budget is set only when something had to be cut.
        #[test]
        fn fitted_columns_stay_within_room(
            room in 0usize..120,
            functions in proptest::collection::vec("[a-z_:]{1,40}", 1..6),
            dirs in proptest::collection::vec("[a-z_]{1,12}", 0..5),
            file in "[a-z_]{1,12}",
        ) {
            let location = format!("{}{file}.rs:12", dirs.iter().flat_map(|d| [d.as_str(), "/"]).collect::<String>());
            let functions: Vec<&str> = functions.iter().map(String::as_str).collect();
            let budgets = fit(room, &functions, &[location.as_str()]);
            let used = column_width(FUNCTION_HEADER, &functions, budgets.function, Cut::End)
                + column_width(LOCATION_HEADER, &[location.as_str()], budgets.location, Cut::Location);
            let floor = column_width(FUNCTION_HEADER, &functions, Some(0), Cut::End)
                + column_width(LOCATION_HEADER, &[location.as_str()], Some(0), Cut::Location);
            if room >= floor {
                prop_assert!(used <= room, "{} > {}", used, room);
            }
            if budgets == Budgets::default() {
                prop_assert!(
                    column_width(FUNCTION_HEADER, &functions, None, Cut::End)
                        + column_width(LOCATION_HEADER, &[location.as_str()], None, Cut::Location)
                        <= room
                );
            }
        }
    }
}

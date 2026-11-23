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
pub(crate) const UNCOVERED_HEADER: &str = "Uncovered";

/// The narrowest the Uncovered column is cut to: its header's width.
pub(crate) const UNCOVERED_FLOOR: usize = 9;
pub(crate) const LOCATION_HEADER: &str = "Location";

/// What a width allows: the coverage bar's cells, the CC column, and the
/// Uncovered column when the hints are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tier {
    pub bar: usize,
    pub cc: bool,
    pub uncovered: bool,
}

/// The columns a width allows. No limit is the full layout.
pub(crate) fn tier(width: Option<usize>) -> Tier {
    let width = width.unwrap_or(usize::MAX);
    let bar = match width {
        100.. => 10,
        80..=99 => 5,
        _ => 0,
    };
    let cc = width >= 60;
    Tier {
        bar,
        cc,
        uncovered: cc,
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

/// Whether the shrinkable columns, cut to their floors, fit `room`. An
/// Uncovered column that is not shown is `None`.
pub(crate) fn can_fit(
    room: usize,
    uncovered: Option<&[&str]>,
    functions: &[&str],
    locations: &[&str],
) -> bool {
    uncovered.map_or(0, |_| UNCOVERED_FLOOR)
        + column_width(FUNCTION_HEADER, functions, Some(0), Cut::End)
        + column_width(LOCATION_HEADER, locations, Some(0), Cut::Location)
        <= room
}

/// How far the Function and Location columns are cut. `None` leaves a
/// column whole.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budgets {
    pub uncovered: Option<usize>,
    pub function: Option<usize>,
    pub location: Option<usize>,
}

/// Fit the shrinkable columns into `room` columns of text. The Uncovered
/// column (`None` when it is not shown) gives way first, down to its
/// header, then Location and Function as [`fit_two`] cuts them.
pub(crate) fn fit(
    room: usize,
    uncovered: Option<&[&str]>,
    functions: &[&str],
    locations: &[&str],
) -> Budgets {
    let others = column_width(FUNCTION_HEADER, functions, None, Cut::End)
        + column_width(LOCATION_HEADER, locations, None, Cut::Location);
    let uncovered_width = |budget| {
        uncovered.map_or(0, |cells| {
            column_width(UNCOVERED_HEADER, cells, budget, Cut::End)
        })
    };
    if uncovered_width(None) + others <= room {
        return Budgets::default();
    }
    let budget = uncovered.map(|_| room.saturating_sub(others).max(UNCOVERED_FLOOR));
    // `fit_two` cuts nothing when the rest already fits.
    let rest = fit_two(
        room.saturating_sub(uncovered_width(budget)),
        functions,
        locations,
    );
    Budgets {
        uncovered: budget,
        ..rest
    }
}

/// Fit the Function and Location columns into `room` columns of text.
/// Location gives way first, down to `…/<file>:<line>`, then Function,
/// down to its header. When even those floors do not fit, both stay at
/// their floors and the table is wider than `room` allows.
fn fit_two(
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
        uncovered: None,
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
        assert_eq!(
            tier(None),
            Tier {
                bar: 10,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(100)),
            Tier {
                bar: 10,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(99)),
            Tier {
                bar: 5,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(80)),
            Tier {
                bar: 5,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(79)),
            Tier {
                bar: 0,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(60)),
            Tier {
                bar: 0,
                cc: true,
                uncovered: true
            }
        );
        assert_eq!(
            tier(Some(59)),
            Tier {
                bar: 0,
                cc: false,
                uncovered: false
            }
        );
        assert_eq!(
            tier(Some(1)),
            Tier {
                bar: 0,
                cc: false,
                uncovered: false
            }
        );
    }

    #[test]
    fn a_table_is_its_content_plus_padding_and_borders() {
        // "│ a ┆ bb │" is 1 + 3 + 1 + 4 + 1 columns.
        assert_eq!(table_width(&[1, 2]), 10);
        assert_eq!(table_width(&[]), 1);
    }

    #[test]
    fn content_that_fits_is_not_cut() {
        let budgets = fit(40, None, &["run"], &["src/main.rs:12"]);
        assert_eq!(budgets, Budgets::default());
    }

    #[test]
    fn location_is_cut_before_function() {
        let budgets = fit(
            40,
            None,
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
            None,
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
        let budgets = fit(27, None, &["run"], &["src/report/pr_comment.rs:380"]);
        assert_eq!(budgets.location, Some(19));
        assert_eq!(budgets.function, None);
    }

    #[test]
    fn uncovered_is_cut_before_location_and_function() {
        let budgets = fit(
            60,
            Some(&["1003, 1005, 1007 +29 more"]),
            &["spans_many_lines"],
            &["src/report/pr_comment.rs:380"],
        );
        // 16 + 28 leaves 16 for Uncovered: cut, the others whole.
        assert_eq!(budgets.uncovered, Some(16));
        assert_eq!((budgets.function, budgets.location), (None, None));
    }

    #[test]
    fn uncovered_stops_at_its_header_before_location_gives_way() {
        let budgets = fit(
            40,
            Some(&["1003, 1005, 1007 +29 more"]),
            &["spans_many_lines"],
            &["src/report/pr_comment.rs:380"],
        );
        // Uncovered at its 9-column header leaves 31: Location goes to 19,
        // and Function (16) to the 12 left.
        assert_eq!(budgets.uncovered, Some(UNCOVERED_FLOOR));
        assert_eq!(budgets.location, Some(19));
        assert_eq!(budgets.function, Some(12));
    }

    #[test]
    fn can_fit_counts_the_uncovered_floor() {
        let functions = ["write_pr_comment_hot_spots"];
        let locations = ["src/report/pr_comment.rs:380"];
        assert!(can_fit(36, Some(&["1003"]), &functions, &locations));
        assert!(!can_fit(35, Some(&["1003"]), &functions, &locations));
    }

    #[test]
    fn neither_column_goes_below_its_header() {
        let budgets = fit(4, None, &["write_pr_comment_hot_spots"], &["lib.rs:3"]);
        assert_eq!(budgets.function, Some(FUNCTION_FLOOR));
        assert_eq!(budgets.location, Some(HEADER_FLOOR));
    }

    #[test]
    fn can_fit_compares_the_floors_with_the_room() {
        let functions = ["write_pr_comment_hot_spots"];
        let locations = ["src/report/pr_comment.rs:380"];
        // Floors: Function 8, Location "…/pr_comment.rs:380" 19.
        assert!(can_fit(27, None, &functions, &locations));
        assert!(!can_fit(26, None, &functions, &locations));
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
            prop_assert!(!narrow.uncovered || wide.uncovered);
        }

        /// The cut columns fit `room` whenever their floors do, and a
        /// budget is set only when something had to be cut.
        #[test]
        fn fitted_columns_stay_within_room(
            room in 0usize..120,
            uncovered in proptest::option::of("[0-9, +a-z–]{0,40}"),
            functions in proptest::collection::vec("[a-z_:]{1,40}", 1..6),
            dirs in proptest::collection::vec("[a-z_]{1,12}", 0..5),
            file in "[a-z_]{1,12}",
        ) {
            let location = format!("{}{file}.rs:12", dirs.iter().flat_map(|d| [d.as_str(), "/"]).collect::<String>());
            let functions: Vec<&str> = functions.iter().map(String::as_str).collect();
            let locations = [location.as_str()];
            let uncovered_cells: Option<Vec<&str>> = uncovered.as_deref().map(|cell| vec![cell]);
            let uncovered = uncovered_cells.as_deref();
            let width = |budgets: Budgets| {
                uncovered.map_or(0, |cells| column_width(UNCOVERED_HEADER, cells, budgets.uncovered, Cut::End))
                    + column_width(FUNCTION_HEADER, &functions, budgets.function, Cut::End)
                    + column_width(LOCATION_HEADER, &locations, budgets.location, Cut::Location)
            };
            let budgets = fit(room, uncovered, &functions, &locations);
            let floors = Budgets { uncovered: Some(0), function: Some(0), location: Some(0) };
            if room >= width(floors) {
                prop_assert!(width(budgets) <= room, "{} > {}", width(budgets), room);
            }
            prop_assert_eq!(can_fit(room, uncovered, &functions, &locations), room >= width(floors));
            if budgets == Budgets::default() {
                prop_assert!(width(Budgets::default()) <= room);
            }
        }
    }
}

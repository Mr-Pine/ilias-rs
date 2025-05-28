use chrono::{DateTime, Days, Local, NaiveTime, TimeZone};
use client::IliasClient;
use regex::Regex;
use reqwest::Url;
use scraper::ElementRef;
use snafu::{OptionExt, ResultExt, Whatever};

pub mod client;
pub mod exercise;
pub mod file;
pub mod folder;
pub mod local_file;
pub mod reference;

pub const ILIAS_URL: &str = "https://ilias.studium.kit.edu";

pub trait IliasElement: Sized {
    fn type_identifier() -> Option<&'static str>;
    fn querypath_from_id(id: &str) -> Option<String>;

    fn parse(element: ElementRef, ilias_client: &IliasClient) -> Result<Self, Whatever>;
}

fn parse_date(date_string: &str) -> Result<DateTime<Local>, Whatever> {
    let (date, time) = date_string.split_once(',').whatever_context(format!(
        "Could not separate date and time in {date_string}"
    ))?;
    let date = date.trim();
    let time = time.trim();

    let time = NaiveTime::parse_from_str(time, "%H:%M")
        .whatever_context(format!("Unable to parse ilias date: {time}"))?;

    let date = if ["Gestern", "Yesterday"].contains(&date) {
        Local::now() - Days::new(1)
    } else if ["Heute", "Today"].contains(&date) {
        Local::now()
    } else if ["Morgen", "Tomorrow"].contains(&date) {
        Local::now() + Days::new(1)
    } else {
        let months: [&[&str]; 12] = [
            &["Jan"],
            &["Feb"],
            &["Mär", "Mar"],
            &["Apr"],
            &["Mai", "May"],
            &["Jun"],
            &["Jul"],
            &["Aug"],
            &["Sep"],
            &["Okt", "Oct"],
            &["Nov"],
            &["Dez", "Dec"],
        ];

        let date_regex = Regex::new(r"^(?<day>\d+)\. (?<month>\w+) (?<year>\w+)$")
            .whatever_context("Could not parse regex")?;
        let date_split = date_regex
            .captures(date)
            .whatever_context(format!("Could not match date {date}"))?;
        let (day, month, year) = (
            date_split.name("day").unwrap().as_str(),
            date_split.name("month").unwrap().as_str(),
            date_split.name("year").unwrap().as_str(),
        );
        let day: u32 = day
            .parse()
            .whatever_context(format!("Could not parse day: {day}"))?;
        let month = months
            .iter()
            .enumerate()
            .find_map(|(index, &names)| {
                if names.contains(&month) {
                    Some(index as u32 + 1)
                } else {
                    None
                }
            })
            .whatever_context(format!("Could not parse month {month}"))?;
        let year: i32 = year
            .parse()
            .whatever_context(format!("Could not parse year: {year}"))?;

        Local
            .with_ymd_and_hms(year, month, day, 0, 0, 0)
            .earliest()
            .whatever_context("Could not construct date")?
    };

    let datetime = date
        .with_time(time)
        .earliest()
        .whatever_context("Could not set time")?;
    Ok(datetime)
}

pub trait Querypath {
    fn get_querypath(&self) -> String;
    fn set_querypath(&mut self, querypath: &str);
}

impl Querypath for Url {
    fn get_querypath(&self) -> String {
        format!("{}?{}", self.path(), self.query().unwrap_or(""))
    }

    fn set_querypath(&mut self, querypath: &str) {
        let mut parts = querypath.split('?');
        self.set_path(parts.next().unwrap());
        self.set_query(parts.next());
    }
}

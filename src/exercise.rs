use std::sync::OnceLock;

use anyhow::{Context, Result};
use assignment::Assignment;
use grades::Grades;
use regex::Regex;
use scraper::{selectable::Selectable, ElementRef, Selector};

pub mod assignment;
pub mod grades;

use super::{client::IliasClient, reference::Reference, IliasElement};

#[derive(Debug)]
#[allow(dead_code)]
pub struct Exercise {
    pub name: String,
    pub description: String,
    pub assignments: Vec<Assignment>,
    pub grades: Reference<Grades>,
}

static ASSIGNMENT_SELECTOR: OnceLock<Selector> = OnceLock::new();
static NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static DESCRIPTION_SELECTOR: OnceLock<Selector> = OnceLock::new();
static GRADES_TAB_SELECTOR: OnceLock<Selector> = OnceLock::new();

static BASE_GRADES_QUERYPATH_REGEX: OnceLock<Regex> = OnceLock::new();

impl IliasElement for Exercise {
    fn type_identifier() -> Option<&'static str> {
        Some("exc")
    }

    fn querypath_from_id(id: &str) -> Option<String> {
        Some(format!(
            "goto.php?target={}_{}&client_id=produktiv",
            Self::type_identifier().unwrap(),
            id
        ))
    }

    fn parse(element: ElementRef, ilias_client: &IliasClient) -> Result<Exercise> {
        let name_selector = NAME_SELECTOR.get_or_init(|| {
            Selector::parse(".il-page-content-header").expect("Could not parse scraper")
        });
        let description_selector = DESCRIPTION_SELECTOR
            .get_or_init(|| Selector::parse(".ilHeaderDesc").expect("Could not parse scraper"));
        let assignment_selector = ASSIGNMENT_SELECTOR.get_or_init(|| {
            Selector::parse("div.il_VAccordionContainer div.il_VAccordionInnerContainer")
                .expect("Could not parse scraper")
        });
        let grades_tab_selector = GRADES_TAB_SELECTOR.get_or_init(|| {
            Selector::parse(".nav-tabs #tab_grades a").expect("Could not parse scraper")
        });

        let base_grades_querypath_regex = BASE_GRADES_QUERYPATH_REGEX
            .get_or_init(|| Regex::new(r".*ref_id=\d+").expect("Could not parse regex"));

        let name = element
            .select(name_selector)
            .next()
            .context(r#"No "name" Element found"#)?
            .text()
            .collect();
        let description = element
            .select(description_selector)
            .next()
            .context(r#"No "description" Element found"#)?
            .text()
            .collect();
        let grades_tab_querypath = element.select(grades_tab_selector).next().map(|link| {
            let querypath = link
                .attr("href")
                .expect("Did not find href on grades tab link")
                .to_string();
            let base_querypath = base_grades_querypath_regex
                .find(&querypath)
                .unwrap_or_else(|| panic!("Grades querypath {querypath} had unexpected format"))
                .as_str()
                .to_string();
            base_querypath
        });
        let assignments = element
            .select(assignment_selector)
            .map(|assignment| {
                Assignment::parse(assignment, ilias_client).expect("Could not parse assignment")
            })
            .collect();

        Ok(Exercise {
            name,
            description,
            assignments,
            grades: Reference::from_optional_querypath(grades_tab_querypath),
        })
    }
}

impl Exercise {
    pub fn get_grades(&mut self, ilias_client: &IliasClient) -> Option<&Grades> {
        let grades = &mut self.grades;
        match grades {
            Reference::Unavailable => None,
            Reference::Resolved(ref grades) => Some(grades),
            Reference::Unresolved(querypath) => {
                let ass_sub = Grades::parse(
                    ilias_client
                        .get_querypath(querypath)
                        .expect("Could not get submission page")
                        .root_element(),
                    querypath,
                )
                .expect("Could not parse submission page");
                *grades = Reference::Resolved(ass_sub);

                grades.try_get_resolved()
            }
        }
    }
}

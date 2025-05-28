use std::{fmt::Display, path::Path, sync::OnceLock};

use base64::Engine;
use regex::Regex;
use scraper::{ElementRef, Html, Selector, selectable::Selectable};
use snafu::{OptionExt, ResultExt, Whatever};
use submission::GradeSubmission;

use crate::{IliasElement, client::IliasClient, reference::Reference};

pub mod submission;

#[derive(Debug)]
pub struct Grades {
    pub assignment_grades: Vec<Reference<GradePage>>,
}

static ASS_ID_OPTION_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl Grades {
    pub fn parse(element: ElementRef, base_querypath: &str) -> Result<Self, Whatever> {
        let ass_id_option_selector = ASS_ID_OPTION_SELECTOR.get_or_init(|| {
            Selector::parse("select#ass_id>option").expect("Could not parrse selector")
        });

        let grade_pages = element
            .select(ass_id_option_selector)
            .map(|option| {
                let ass_id = option.attr("value").expect("Option did not have a value");

                let querypath = format!("{base_querypath}&ass_id={ass_id}");
                Reference::Unresolved(querypath)
            })
            .collect::<Vec<_>>();

        Ok(Grades {
            assignment_grades: grade_pages,
        })
    }
}

#[derive(Debug)]
pub struct GradePage {
    pub name: String,
    ass_id: String,
    toolbar_form_querypath: String,
    pub submissions: Vec<GradeSubmission>,
}

static SELECTED_ASSIGNMENT_DROPDOWN_SELECTOR: OnceLock<Selector> = OnceLock::new();
static TOOLBAR_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();
static SUBMISSION_ROW_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl IliasElement for GradePage {
    fn type_identifier() -> Option<&'static str> {
        None
    }

    fn querypath_from_id(_id: &str) -> Option<String> {
        None
    }

    fn parse(element: ElementRef, _ilias_client: &IliasClient) -> Result<Self, Whatever> {
        let selected_assignment_dropdown_selector = SELECTED_ASSIGNMENT_DROPDOWN_SELECTOR
            .get_or_init(|| {
                Selector::parse(r#"select#ass_id option[selected="selected"]"#)
                    .expect("Could not parse selector")
            });
        let toolbar_form_selector = TOOLBAR_FORM_SELECTOR
            .get_or_init(|| Selector::parse("form#ilToolbar").expect("Could not parse selector"));
        let submission_row_selector = SUBMISSION_ROW_SELECTOR.get_or_init(|| {
            Selector::parse("table#exc_mem tbody tr").expect("Could not parse selector")
        });

        let assignment_selection = element
            .select(selected_assignment_dropdown_selector)
            .next()
            .whatever_context("Did not find selected assignment in dropdown")?;
        let ass_id = assignment_selection
            .attr("value")
            .whatever_context("Dropdown entry did not have a value")?
            .to_string();
        let name = assignment_selection.text().collect();

        let toolbar_form_querypath = element
            .select(toolbar_form_selector)
            .next()
            .whatever_context("Did not find toolbar form")?
            .attr("action")
            .whatever_context("Toolbar form had no action")?
            .to_string();

        let mut submissions = vec![];
        for submission_element in element.select(submission_row_selector) {
            if let Some(submission) = GradeSubmission::parse(submission_element)
                .whatever_context("Could not parse submission")?
            {
                submissions.push(submission);
            }
        }

        Ok(GradePage {
            name,
            ass_id,
            toolbar_form_querypath,
            submissions,
        })
    }
}

static NOTIFICATION_ITEM_BUTTON_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl GradePage {
    pub fn download_all_submissions_zip(
        &self,
        ilias_client: &IliasClient,
        to: &Path,
    ) -> Result<(), Whatever> {
        let form_data = [
            ("ass_id", self.ass_id.as_str()),
            ("user_login", ""),
            ("cmd[downloadSubmissions]", ":)"),
        ];
        let response =
            ilias_client.post_querypath_form(&self.toolbar_form_querypath, &form_data)?;
        let html = Html::parse_document(&ilias_client.get_text(response)?);

        let notification_item_button_selector = NOTIFICATION_ITEM_BUTTON_SELECTOR.get_or_init(|| Selector::parse(".il-aggregate-notifications .il-notification-item .media-body .il-item-notification-title button").expect("Could not parse selector"));
        let from_url_regex =
            Regex::new("from_url=(?<url>[^&]+)&").whatever_context("Unable to parse regex")?;
        let dowload_querypath = html
            .select(notification_item_button_selector)
            .map(|button| button.attr("data-action").expect("Button had no action"))
            .find_map(|querypath| {
                let form_url = from_url_regex.captures(querypath)?.name("url")?.as_str();
                let form_url = String::from_utf8(
                    base64::prelude::BASE64_URL_SAFE_NO_PAD
                        .decode(form_url)
                        .ok()?,
                )
                .ok()?;

                if form_url.contains(&self.ass_id) {
                    Some(querypath)
                } else {
                    None
                }
            })
            .whatever_context("Could not find download querypath")?;

        ilias_client.download_file(dowload_querypath, to)?;

        Ok(())
    }

    pub fn update_points(
        &self,
        ilias_client: &IliasClient,
        changed_submissions: &Vec<GradeSubmission>,
    ) -> Result<(), Whatever> {
        let form_data = [
            ("flt_status", ""),
            ("flt_subm", ""),
            ("flt_subm_after", ""),
            ("flt_subm_before", ""),
            ("tblfsexc_mem[]", "login"),
            ("tblfsexc_mem[]", "submission"),
            ("tblfsexc_mem[]", "idl"),
            ("tblfsexc_mem[]", "mark"),
            ("tblfshexc_mem", "1"),
            ("tbltplcrt", ""),
            ("selected_cmd", "saveStatusSelected"),
            ("selected_cmd2", "saveStatusSelected"),
            ("select_cmd2", "Ausführen"),
        ];
        let mut form_data = form_data.map(|(a, b)| (a.to_string(), b)).to_vec();

        for submission in changed_submissions {
            form_data.push(("sel_part_ids[]".to_string(), &submission.ilias_id));
            form_data.push(("listed_part_ids[]".to_string(), &submission.ilias_id));
            form_data.push((format!("status[{}]", &submission.ilias_id), "notgraded"));
            form_data.push((
                format!("mark[{}]", &submission.ilias_id),
                &submission.points,
            ));
        }
        ilias_client.post_querypath_form(&self.toolbar_form_querypath, &form_data)?;
        Ok(())
    }
}

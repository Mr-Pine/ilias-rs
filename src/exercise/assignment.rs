use std::sync::OnceLock;

use chrono::{DateTime, Local};
use log::debug;
use regex::Regex;
use reqwest::multipart::Form;
use scraper::{selectable::Selectable, ElementRef, Selector};
use snafu::{OptionExt, ResultExt, Whatever};

use crate::reference::Reference;

use super::super::{
    client::{AddFileWithFilename, IliasClient},
    file::File,
    local_file::NamedLocalFile,
    parse_date, IliasElement,
};

#[derive(Debug)]
#[allow(dead_code)]
pub struct Assignment {
    pub name: String,
    pub instructions: Option<String>,
    pub submission_start_date: Option<DateTime<Local>>,
    pub submission_end_date: Option<DateTime<Local>>,
    pub attachments: Vec<File>,
    submission: Reference<AssignmentSubmission>,
}

static PANEL_SELECTOR: OnceLock<Selector> = OnceLock::new();
static PANEL_NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static PANEL_BODY_SELECTOR: OnceLock<Selector> = OnceLock::new();

static NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static PROPERTY_ROW_SELECTOR: OnceLock<Selector> = OnceLock::new();
static ATTACHMENT_ROW_SELECTOR: OnceLock<Selector> = OnceLock::new();
static SUBMISSION_PAGE_SELECTOR: OnceLock<Selector> = OnceLock::new();
static INFO_PROPERTY_VALUE_SELECTOR: OnceLock<Selector> = OnceLock::new();
static INFO_PROPERTY_KEY_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl IliasElement for Assignment {
    fn type_identifier() -> Option<&'static str> {
        Some("ass")
    }

    fn querypath_from_id(_: &str) -> Option<String> {
        None
    }

    fn parse(element: ElementRef, ilias_client: &IliasClient) -> Result<Self, Whatever> {
        let name_selector = NAME_SELECTOR.get_or_init(|| {
            Selector::parse(".il-item-title > a").expect("Could not parse selector")
        });
        let panel_selector = PANEL_SELECTOR
            .get_or_init(|| Selector::parse(".panel.panel-sub").expect("Could not parse selector"));
        let panel_name_selector = PANEL_NAME_SELECTOR
            .get_or_init(|| Selector::parse("h3").expect("Could not parse selector"));
        let panel_body_selector = PANEL_BODY_SELECTOR
            .get_or_init(|| Selector::parse(".panel-body").expect("Could not parse selector"));

        let submission_page_selector = SUBMISSION_PAGE_SELECTOR.get_or_init(|| {
            Selector::parse("#tab_submission > a").expect("Could not parse selector")
        });
        let property_row_selector = PROPERTY_ROW_SELECTOR.get_or_init(|| {
            Selector::parse(".il-multi-line-cap-3").expect("Could not parse selector")
        });
        let attachment_row_selector = ATTACHMENT_ROW_SELECTOR
            .get_or_init(|| Selector::parse(".row").expect("Could not parse selector"));

        let name: String = element
            .select(name_selector)
            .next()
            .whatever_context("Did not find name")?
            .text()
            .collect();
        debug!("Assignment name: {name}");

        let properties: Vec<_> = element.select(property_row_selector).collect();
        debug!("Properties: {properties:?}");

        let submission_start_date =
            Self::get_value_for_keys(&properties, &["Startzeit", "Start Time"])
                .ok()
                .map(|date| parse_date(date.trim()))
                .transpose()?;
        let submission_end_date =
            Self::get_value_for_keys(&properties, &["Abgabetermin", "Edit Until"])
                .or_else(|_| Self::get_value_for_keys(&properties, &["Beendet am", "Ended On"]))
                .and_then(|date| parse_date(date.trim()))
                .ok();
        debug!("Start: {submission_start_date:?}; End: {submission_end_date:?}");

        let detail_querypath = element
            .select(name_selector)
            .next()
            .whatever_context("Did not find name element for detail querypath")?
            .attr("href")
            .whatever_context("Could not get href attr for detail querypath")?;
        let detail_page = ilias_client
            .get_querypath(detail_querypath)
            .whatever_context("Could not get detail html")?;

        let panels: Vec<_> = detail_page.select(panel_selector).collect();

        let instruction_panel = panels.iter().find(|panel| {
            panel
                .select(panel_name_selector)
                .next()
                .map(|name| {
                    ["Arbeitsanweisung", "Work Instructions"]
                        .contains(&name.text().collect::<String>().as_str())
                })
                .unwrap_or(false)
        });
        let instructions = if let Some(panel) = instruction_panel {
            let body = panel
                .select(panel_body_selector)
                .next()
                .and_then(|body| body.child_elements().next())
                .and_then(|body| body.child_elements().next())
                .whatever_context("Could not get body for instruction panel")?;
            Some(body.text().collect::<String>().trim().to_string())
        } else {
            None
        };
        debug!("Instructions: {instructions:?}");

        let attachment_panel = panels.iter().find(|panel| {
            panel
                .select(panel_name_selector)
                .next()
                .map(|name| {
                    ["Dateien", "Files"].contains(&name.text().collect::<String>().as_str())
                })
                .unwrap_or(false)
        });
        let attachments = if let Some(panel) = attachment_panel {
            let file_rows: Vec<_> = panel.select(attachment_row_selector).collect();
            let mut attachments = vec![];

            for row in &file_rows[0..file_rows.len() - 2] {
                let mut children = row.child_elements();
                let filename = children
                    .next()
                    .whatever_context("Could not get attachment filename")?
                    .text()
                    .collect::<String>()
                    .trim()
                    .to_string();
                let download_querypath = children
                    .next()
                    .and_then(|div| div.child_elements().next())
                    .and_then(|p| p.child_elements().next());
                let download_querypath = download_querypath
                    .whatever_context("Did not find download querypath for attachment")?
                    .attr("href")
                    .whatever_context("Did not find download href for attachment")?;

                let file = File {
                    name: filename,
                    description: "".to_string(),
                    download_querypath: Some(download_querypath.to_string()),
                    date: None,
                    id: None,
                };

                attachments.push(file);
            }

            attachments
        } else {
            vec![]
        };
        debug!("Attachments: {attachments:?}");

        let submission_page_querypath = dbg!(detail_page.select(submission_page_selector).next())
            .and_then(|link| link.attr("href"))
            .map(|querypath| querypath.to_string());

        Ok(Assignment {
            name,
            instructions,
            submission_start_date,
            submission_end_date,
            attachments,
            submission: Reference::from_optional_querypath(submission_page_querypath),
        })
    }
}

impl Assignment {
    pub fn is_active(&self) -> bool {
        self.submission_end_date
            .map_or(true, |date| date >= Local::now())
            && self
                .submission_start_date
                .map_or(true, |date| date <= Local::now())
    }

    pub fn get_submission(
        &mut self,
        ilias_client: &IliasClient,
    ) -> Result<Option<&AssignmentSubmission>, Whatever> {
        let submission = &mut self.submission;
        let res = match submission {
            Reference::Unavailable => None,
            Reference::Resolved(ref submission) => Some(submission),
            Reference::Unresolved(querypath) => {
                let ass_sub = AssignmentSubmission::parse_submissions_page(
                    ilias_client
                        .get_querypath(querypath)
                        .whatever_context("Could not get submission page")?
                        .root_element(),
                    ilias_client,
                )
                .whatever_context("Could not parse submission page")?;
                *submission = Reference::Resolved(ass_sub);

                submission.try_get_resolved()
            }
        };
        Ok(res)
    }

    fn get_value_element_for_keys<'a>(
        properties: &[ElementRef<'a>],
        keys: &[&str],
    ) -> Result<ElementRef<'a>, Whatever> {
        let info_property_value_selector = INFO_PROPERTY_VALUE_SELECTOR.get_or_init(|| {
            Selector::parse(".il-item-property-value").expect("Could not parse selector")
        });
        let info_property_key_selector = INFO_PROPERTY_KEY_SELECTOR.get_or_init(|| {
            Selector::parse(".il-item-property-name").expect("Could not parse selector")
        });

        let property_row = properties
            .iter()
            .find(|&element| {
                keys.contains(
                    &element
                        .select(info_property_key_selector)
                        .next()
                        .expect("Property without key")
                        .text()
                        .collect::<String>()
                        .as_str(),
                )
            })
            .whatever_context(format!("Did not find {:?} property", keys))?;
        property_row
            .select(info_property_value_selector)
            .next()
            .whatever_context(format!("Did not find value for {:?} property", keys))
    }

    fn get_value_for_keys(info_screen: &[ElementRef], keys: &[&str]) -> Result<String, Whatever> {
        Ok(Self::get_value_element_for_keys(info_screen, keys)
            .whatever_context("Could not get key values")?
            .text()
            .collect())
    }
}

#[derive(Debug)]
pub struct AssignmentSubmission {
    pub submissions: Vec<File>,
    delete_querypath: String,
    upload_querypath: String,
}

static UPLOAD_BUTTON_SELECTOR: OnceLock<Selector> = OnceLock::new();
static CONTENT_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();
static FILE_ROW_SELECTOR: OnceLock<Selector> = OnceLock::new();
static SOURCE_TAG_SELECTOR: OnceLock<Selector> = OnceLock::new();

static UPLOAD_QUERYPATH_REGEX: OnceLock<Regex> = OnceLock::new();

impl AssignmentSubmission {
    fn parse_submissions_page(
        submission_page: ElementRef,
        ilias_client: &IliasClient,
    ) -> Result<AssignmentSubmission, Whatever> {
        let upload_button_selector = UPLOAD_BUTTON_SELECTOR.get_or_init(|| {
            Selector::parse(".navbar-form button").expect("Could not parse selector")
        });
        let content_form_selector = CONTENT_FORM_SELECTOR.get_or_init(|| {
            Selector::parse("div#ilContentContainer form").expect("Could not parse selector")
        });
        let source_tag_selector = SOURCE_TAG_SELECTOR.get_or_init(|| {
            Selector::parse("body > script:not([src])").expect("Could not parse selector")
        });
        let file_row_selector = FILE_ROW_SELECTOR.get_or_init(|| {
            Selector::parse("#ilContentContainer form tbody tr").expect("Could not parse selector")
        });
        let upload_querypath_regex = UPLOAD_QUERYPATH_REGEX.get_or_init(|| {
            Regex::new(r#"'(?P<querypath>ilias\.php\?[a-zA-Z=&0-9:_]+cmd=upload[a-zA-Z=&0-9:_]+)'"#)
                .expect("Could not parse regex")
        });

        let file_rows = submission_page.select(file_row_selector);
        let mut uploaded_files = vec![];
        for row in file_rows.filter(|&row| row.child_elements().count() > 1) {
            let mut children = row.child_elements();

            let id = children
                .next()
                .whatever_context("Did not find first column in table")?
                .child_elements()
                .next()
                .whatever_context("Did not find checkbox")?
                .attr("value")
                .whatever_context("Did not find id")?;
            let file_name = children
                .next()
                .whatever_context("Did not find second column")?
                .text()
                .collect();
            let submission_date = loop {
                let parsed_date = parse_date(
                    &children
                        .next()
                        .whatever_context("Did not find date column")?
                        .text()
                        .collect::<String>(),
                );
                match parsed_date {
                    Ok(date) => break date,
                    _ => continue,
                }
            };
            let download_querypath = children
                .last()
                .whatever_context("Did not find last column")?
                .child_elements()
                .next()
                .whatever_context("Did not find download link")?
                .attr("href")
                .whatever_context("Did not find href attribute")?;

            let file = File {
                id: Some(id.to_string()),
                name: file_name,
                description: String::new(),
                date: Some(submission_date),
                download_querypath: Some(download_querypath.to_string()),
            };

            uploaded_files.push(file);
        }

        let delete_querypath = submission_page
            .select(content_form_selector)
            .next()
            .whatever_context("Did not find deletion form")?
            .value()
            .attr("action")
            .whatever_context("Did not find action attribute for delete querypath")?
            .to_string();

        let upload_form_querypath = submission_page
            .select(upload_button_selector)
            .next()
            .whatever_context("Did not find upload button")?
            .attr("data-action")
            .whatever_context("Did not find data-action on upload button")?;
        debug!("Upload form querypath: {}", upload_form_querypath);
        let upload_page = ilias_client.get_querypath(upload_form_querypath)?;
        let script = upload_page
            .select(source_tag_selector)
            .next()
            .whatever_context("Missing script with upload path")?
            .text()
            .collect::<String>();
        let upload_querypath = upload_querypath_regex
            .captures(&script)
            .whatever_context("Could not find upload querypath")?["querypath"]
            .to_string();
        debug!("Upload querypath: {}", upload_querypath);

        Ok(AssignmentSubmission {
            submissions: uploaded_files,
            delete_querypath,
            upload_querypath,
        })
    }

    pub fn delete_files(
        &self,
        ilias_client: &IliasClient,
        files: &[&File],
    ) -> Result<(), Whatever> {
        let mut form_args = files
            .iter()
            .map(|&file| file.id.clone().expect("Files to delete must have an id"))
            .map(|id| ("delivered[]", id))
            .collect::<Vec<_>>();
        form_args.push(("cmd[deleteDelivered]", String::from("Löschen")));

        ilias_client
            .post_querypath_form(&self.delete_querypath, &form_args)
            .whatever_context("Could not post assignment deletion form")?;
        Ok(())
    }

    pub fn upload_files(
        &self,
        ilias_client: &IliasClient,
        files: &[NamedLocalFile],
    ) -> Result<(), Whatever> {
        let mut form = Form::new();

        for (index, file_data) in files.iter().enumerate() {
            form = form
                .file_with_name(
                    format!("deliver[{index}]"),
                    ilias_client.construct_file_part(&file_data.path),
                    file_data.name.clone(),
                )?
                .text("cmd[uploadFile]", "Hochladen")
                .text("ilfilehash", "aaaa");
        }
        debug!("Form: {:?}", form);
        debug!("Upload querypath: {}", self.upload_querypath);

        ilias_client
            .post_querypath_multipart(&self.upload_querypath, form)
            .whatever_context("Could not post assignment upload form")?;
        Ok(())
        // TODO: Maybe push files to submission here
    }
}

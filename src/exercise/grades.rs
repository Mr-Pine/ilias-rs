use std::{path::Path, sync::OnceLock};

use anyhow::{Context, Result};
use base64::Engine;
use regex::Regex;
use scraper::{selectable::Selectable, ElementRef, Html, Selector};

use crate::{client::IliasClient, IliasElement, Reference};

#[derive(Debug)]
pub struct Grades {
    pub assignment_grades: Vec<Reference<GradePage>>,
}

static ASS_ID_OPTION_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl Grades {
    fn parse(
        element: ElementRef,
        base_querypath: &str,
        ilias_client: &IliasClient,
    ) -> Result<Self> {
        let ass_id_option_selector = ASS_ID_OPTION_SELECTOR.get_or_init(|| {
            Selector::parse("select#ass_id>option").expect("Could not parrse selector")
        });

        let grade_pages = element
            .select(ass_id_option_selector)
            .map(|option| {
                let ass_id = option.attr("value").expect("Option did not have a value");
                let selected = option
                    .attr("selected")
                    .map_or(false, |val| val == "selected");

                if selected {
                    Reference::Resolved(
                        GradePage::parse(element, ilias_client)
                            .expect("Could not parse grade page"),
                    )
                } else {
                    let querypath = format!("{}&ass_id={}", base_querypath, ass_id);
                    Reference::Unresolved(querypath)
                }
            })
            .collect::<Vec<_>>();

        Ok(Grades {
            assignment_grades: grade_pages,
        })
    }
}

#[derive(Debug)]
pub struct GradePage {
    ass_id: String,
    toolbar_form_querypath: String,
}

static TOOLBAR_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl IliasElement for GradePage {
    fn type_identifier() -> Option<&'static str> {
        None
    }

    fn querypath_from_id(_id: &str) -> Option<String> {
        None
    }

    fn parse(element: ElementRef, ilias_client: &IliasClient) -> Result<Self> {
        let selected_assignment_dropdown_selector = TOOLBAR_FORM_SELECTOR.get_or_init(|| {
            Selector::parse("select#ass_id option[selected=\"selected\"]")
                .expect("Could not parse selector")
        });
        let toolbar_form_selector = TOOLBAR_FORM_SELECTOR
            .get_or_init(|| Selector::parse("form#ilToolbar").expect("Could not parse selector"));

        let ass_id = element
            .select(selected_assignment_dropdown_selector)
            .next()
            .context("Did not find selected assignment in dropdown")?
            .attr("value")
            .context("Dropdown entry did not have a value")?
            .to_string();
        let toolbar_form_querypath = element
            .select(toolbar_form_selector)
            .next()
            .context("Did not find toolbar form")?
            .attr("action")
            .context("Toolbar form had no action")?
            .to_string();

        Ok(GradePage {
            ass_id,
            toolbar_form_querypath,
        })
    }
}

static NOTIFICATION_ITEM_BUTTON_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl GradePage {
    fn download_all_submissions_zip(&self, ilias_client: &IliasClient, to: &Path) -> Result<()> {
        let form_data = [
            ("ass_id", self.ass_id.as_str()),
            ("user_login", ""),
            ("cmd[downloadSubmissions]", ":)"),
        ];
        let response =
            ilias_client.post_querypath_form(&self.toolbar_form_querypath, &form_data)?;
        let html = Html::parse_document(&ilias_client.get_text(response)?);

        let notification_item_button_selector = NOTIFICATION_ITEM_BUTTON_SELECTOR.get_or_init(|| Selector::parse(".il-aggregate-notifications .il-notification-item .media-body .il-item-notification-title button").expect("Could not parse selector"));
        let form_url_regex = Regex::new("form_url=(?<url>[^&]+)&")?;
        let dowload_querypath = html
            .select(notification_item_button_selector)
            .map(|button| button.attr("data-action").expect("Button had no action"))
            .find_map(|querypath| {
                let form_url = form_url_regex.captures(querypath)?.name("url")?.as_str();
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
            .context("Could not find download querypath")?;

        todo!()
    }
}

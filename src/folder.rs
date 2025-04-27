use std::{fmt::Display, sync::OnceLock};

use log::debug;
use regex::Regex;
use reqwest::{multipart::Form, Url};
use scraper::{element_ref::Select, selectable::Selectable, ElementRef, Selector};
use serde::{Deserialize, Serialize};
use snafu::{whatever, OptionExt, ResultExt, Whatever};

use super::{
    client::IliasClient, file::File, local_file::NamedLocalFile, parse_date, IliasElement,
    Querypath,
};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum FolderElement {
    File {
        file: File,
        deletion_querypath: Option<String>,
    },
    Exercise {
        name: String,
        description: String,
        id: String,
        querypath: String,
        deletion_querypath: Option<String>,
    },
    Opencast {
        name: String,
        description: String,
        id: String,
        querypath: String,
        deletion_querypath: Option<String>,
    },
    Viewable {
        name: String,
        description: String,
        id: String,
        querypath: String,
        deletion_querypath: Option<String>,
    },
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct Folder {
    name: String,
    description: String,
    id: String,
    pub elements: Vec<FolderElement>,
    upload_page_querypath: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IliasUploadResponse {
    status: u8,
    message: String,
    file_id: String,
}

static NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static DESCRIPTION_SELECTOR: OnceLock<Selector> = OnceLock::new();
static ID_SELECTOR: OnceLock<Selector> = OnceLock::new();
static UPLOAD_FILE_PAGE_SELECTOR: OnceLock<Selector> = OnceLock::new();

static ELEMENT_SELECTOR: OnceLock<Selector> = OnceLock::new();
static LAST_SCRIPT_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl IliasElement for Folder {
    fn type_identifier() -> Option<&'static str> {
        Some("fold")
    }

    fn querypath_from_id(id: &str) -> Option<String> {
        Some(format!(
            "goto.php/{}/{}",
            Self::type_identifier().unwrap(),
            id
        ))
    }

    fn parse(element: ElementRef, ilias_client: &IliasClient) -> Result<Self, Whatever> {
        let name_selector = NAME_SELECTOR.get_or_init(|| {
            Selector::parse(".il-page-content-header").expect("Could not parse selector")
        });
        let description_selector = DESCRIPTION_SELECTOR
            .get_or_init(|| Selector::parse(".ilHeaderDesc").expect("Could not parse selector"));
        let id_selector = ID_SELECTOR.get_or_init(|| {
            Selector::parse(".breadcrumbs span:last-child a").expect("Could not parse selector")
        });
        let upload_file_page_selector = UPLOAD_FILE_PAGE_SELECTOR.get_or_init(|| {
            Selector::parse("#il-add-new-item-gl #file").expect("Could not parse selector")
        });

        let element_selector = ELEMENT_SELECTOR.get_or_init(|| {
            Selector::parse(".ilContainerListItemContent").expect("Could not parse selector")
        });
        let last_script_selector = LAST_SCRIPT_SELECTOR.get_or_init(|| {
            Selector::parse("body script:last-child").expect("Could not parse selector")
        });

        let name = element
            .select(name_selector)
            .next()
            .whatever_context("Could not find name")?
            .text()
            .collect();
        let description = element
            .select(description_selector)
            .next()
            .whatever_context("Could not find description")?
            .text()
            .collect();
        let id = element
            .select(id_selector)
            .next()
            .whatever_context("Could not find link in breadcrumbs")?
            .attr("href")
            .whatever_context("Link missing href attribute")?
            .to_string();

        let last_script = element
            .select(last_script_selector)
            .next()
            .whatever_context("Did not find last script")?
            .text()
            .collect::<String>();

        let elements: Vec<FolderElement> = element
            .select(element_selector)
            .filter_map(|element| FolderElement::parse(element, &last_script, ilias_client))
            .collect();

        let upload_page_querypath = element
            .select(upload_file_page_selector)
            .next()
            .and_then(|link| link.attr("href"))
            .map(str::to_string);

        Ok(Folder {
            name,
            description,
            id,
            elements,
            upload_page_querypath,
        })
    }
}

static CONTENT_FORM_SELECTOR: OnceLock<Selector> = OnceLock::new();
static SCRIPT_TAG_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl Folder {
    pub fn upload_files(
        &self,
        ilias_client: &IliasClient,
        files: &[NamedLocalFile],
    ) -> Result<(), Whatever> {
        debug!(
            "Uploading files: {:?} to {:?}",
            files, &self.upload_page_querypath
        );
        let upload_page = ilias_client.get_querypath(
            &self
                .upload_page_querypath
                .clone()
                .whatever_context("No upload available for this folder")?,
        )?;
        let upload_form_selector = CONTENT_FORM_SELECTOR.get_or_init(|| {
            Selector::parse("#ilContentContainer form").expect("Could not parse scraper")
        });
        let script_tag_selector = SCRIPT_TAG_SELECTOR.get_or_init(|| {
            Selector::parse("body script:not([src])").expect("Could not parse scraper")
        });

        let finish_upload_querypath = upload_page
            .select(upload_form_selector)
            .next()
            .unwrap()
            .value()
            .attr("action")
            .unwrap();
        debug!("Finish upload querypath: {}", finish_upload_querypath);

        let relevant_script_tag = upload_page
            .select(script_tag_selector)
            .next()
            .unwrap()
            .text()
            .collect::<String>();

        let path_regex =
            Regex::new(r".*il\.UI\.Input\.File\.init\([^']*'[^']*',[^']*'(?<querypath>[^']+)'.*")
                .whatever_context("Could not parse cursed regex lol")?;
        let upload_querypath = &path_regex
            .captures(&relevant_script_tag)
            .whatever_context("No match for upload querypath found :(")?["querypath"];
        debug!("Upload querypath: {}", upload_querypath);

        for file_data in files {
            let form = Form::new().part(
                "file[0]",
                ilias_client.construct_file_part(&file_data.path)?,
            );

            let response = ilias_client.post_querypath_multipart(upload_querypath, form)?;
            let response: IliasUploadResponse = ilias_client.get_json(response)?;
            debug!("Upload response: {response:?}");
            let file_id = response.file_id;

            let finish_form = Form::new()
                .text("form/input_0[input_1][]", file_data.name.clone()) // Filename
                .text("form/input_0[input_2][]", "") // Description
                .text("form/input_0[input_3][]", file_id) // File id
                .text("form/input_1", "7") // License: All rights reserved
                .percent_encode_noop();

            let response =
                ilias_client.post_querypath_multipart(finish_upload_querypath, finish_form)?;
            debug!("Finish upload response: {:?}", response);
            if ilias_client
                .is_alert_response(response)
                .whatever_context("Could not check error state of response")?
            {
                whatever!("Upload response has an error, please check if the file was uploaded and report")
            }
        }

        Ok(())
        // TODO: Maybe push files to submission here
    }
}

static ELEMENT_NAME_SELECTOR: OnceLock<Selector> = OnceLock::new();
static ELEMENT_DESCRIPTION_SELECTOR: OnceLock<Selector> = OnceLock::new();
static ELEMENT_ACTIONS_SELECTOR: OnceLock<Selector> = OnceLock::new();
static ELEMENT_PROPERTY_SELECTOR: OnceLock<Selector> = OnceLock::new();

impl FolderElement {
    fn parse(
        element: ElementRef,
        folder_script: &str,
        ilias_client: &IliasClient,
    ) -> Option<FolderElement> {
        let element_name_selector = ELEMENT_NAME_SELECTOR.get_or_init(|| {
            Selector::parse(".il_ContainerItemTitle a").expect("Could not parse selector")
        });
        let element_description_selector = ELEMENT_DESCRIPTION_SELECTOR
            .get_or_init(|| Selector::parse(".il_Description").expect("Could not parse selector"));
        let element_property_selector = ELEMENT_PROPERTY_SELECTOR
            .get_or_init(|| Selector::parse(".il_ItemProperty").expect("Could not parse selector"));

        let name_element = element.select(element_name_selector).next()?;
        let description_element = element.select(element_description_selector).next()?;
        let mut properties = element.select(element_property_selector);

        let name: String = name_element.text().collect();
        let link = name_element.attr("href")?;
        let description = description_element.text().collect();
        let querypath = Url::parse(link)
            .expect("Could not parse link")
            .get_querypath();

        let id = Regex::new(r"ref_id=(?<id>\d+)|target=file_(?<file_id>\d+)")
            .ok()?
            .captures(&querypath)
            .and_then(|capture| capture.name("id").or(capture.name("file_id")))?
            .as_str()
            .to_string();

        let deletion_querypath = Self::get_deletion_querypath(&id, folder_script, ilias_client);

        Self::extract_from_querypath(
            querypath,
            name,
            description,
            id,
            deletion_querypath,
            &mut properties,
        )
    }

    fn get_deletion_querypath(
        id: &str,
        folder_script: &str,
        ilias_client: &IliasClient,
    ) -> Option<String> {
        let element_actions_selector = ELEMENT_ACTIONS_SELECTOR
            .get_or_init(|| Selector::parse("li>a").expect("Could not parse selector"));

        let regex = format!(
            r##"\$\("#ilAdvSelListAnchorText_act_{}_pref_\d+"\).click\((?:.|\n)*ajaxReplaceInner\('(?<querypath>[^']+)', 'ilAdvSelListTable_act_{}"##,
            &id, &id
        );
        let actions_querypath = Regex::new(&regex)
            .ok()?
            .captures(folder_script)
            .and_then(|captures| Some(captures.name("querypath")?.as_str().to_string()))?;
        let actions = ilias_client.get_querypath(&actions_querypath).ok()?;

        let deletion_querypath = actions
            .select(element_actions_selector)
            .filter_map(|element| element.attr("href"))
            .find(|&href| href.contains("cmd=delete"))
            .map(ToOwned::to_owned);

        deletion_querypath
    }

    fn extract_from_querypath(
        querypath: String,
        name: String,
        description: String,
        id: String,
        deletion_querypath: Option<String>,
        properties: &mut Select<'_, '_>,
    ) -> Option<FolderElement> {
        if querypath.contains("target=file_") {
            let extension: String = properties
                .next()
                .expect("Could not find file extension")
                .text()
                .collect::<String>()
                .trim()
                .to_string();
            let date = loop {
                let next_property = properties.next();
                match next_property {
                    Some(property) => {
                        let date = parse_date(&property.text().collect::<String>());
                        match date {
                            Ok(date) => break Some(date),
                            Err(_) => continue,
                        }
                    }
                    None => break None,
                }
            };

            let name = if extension.is_empty() {
                name
            } else {
                format!("{name}.{extension}")
            };

            let file = File {
                name,
                description,
                date,
                id: Some(id.to_string()),
                download_querypath: Some(querypath),
            };

            Some(FolderElement::File {
                file,
                deletion_querypath,
            })
        } else if querypath.contains("baseClass=ilObjPluginDispatchGUI")
            && querypath.contains("cmd=forward")
            && querypath.contains("forwardCmd=showContent")
        {
            Some(FolderElement::Opencast {
                name,
                description,
                id,
                querypath,
                deletion_querypath,
            })
        } else if querypath.contains("baseClass=ilrepositorygui") && querypath.contains("cmd=view")
        {
            let id = Regex::new(r"ref_id=(?<id>\d+)")
                .ok()?
                .captures(&querypath)?
                .name("id")?
                .as_str()
                .to_string();
            Some(FolderElement::Viewable {
                name,
                description,
                id,
                querypath,
                deletion_querypath,
            })
        } else {
            None
        }
    }

    fn deletion_querypath(&self) -> Option<&String> {
        match self {
            Self::File {
                deletion_querypath, ..
            }
            | Self::Exercise {
                deletion_querypath, ..
            }
            | Self::Opencast {
                deletion_querypath, ..
            }
            | Self::Viewable {
                deletion_querypath, ..
            } => deletion_querypath,
        }
        .as_ref()
    }

    pub fn file(&self) -> Option<&File> {
        match self {
            Self::File {
                file,
                deletion_querypath: _,
            } => Some(file),
            _ => None,
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::File { file, .. } => file.id.as_ref().unwrap(),
            Self::Exercise { id, .. } | Self::Opencast { id, .. } | Self::Viewable { id, .. } => id,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::File { file, .. } => &file.name,
            Self::Exercise { name, .. }
            | Self::Opencast { name, .. }
            | Self::Viewable { name, .. } => name,
        }
    }

    pub fn delete(&self, ilias_client: &IliasClient) -> Result<(), Whatever> {
        let deletion_querypath = self.deletion_querypath();
        let delete_page =
            ilias_client
                .get_querypath(deletion_querypath.whatever_context(format!(
                    "You can not delete this element: {}",
                    self.name()
                ))?)
                .whatever_context(format!("Error getting delete page for {:?}", self))?;

        let form_selector = CONTENT_FORM_SELECTOR.get_or_init(|| {
            Selector::parse("#ilContentContainer form").expect("Could not parse scraper")
        });
        let confirm_querypath = delete_page
            .select(form_selector)
            .next()
            .whatever_context("Could not find confirmation form")?
            .value()
            .attr("action")
            .whatever_context("Could not find action on form")?;

        let form_data = [
            ("id[]", self.id()),
            ("cmd[confirmedDelete]", "I fucking hate ILIAS"),
        ];

        ilias_client
            .post_querypath_form(confirm_querypath, &form_data)
            .whatever_context(format!(
                "Error while submitting delete confirmation for {:?}",
                self
            ))?;
        println!(
            "Deleted {} via deletion querypath {:?}",
            self.id(),
            deletion_querypath
        );
        Ok(())
    }
}

impl Display for FolderElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FolderElement::File {
                file,
                deletion_querypath: _,
            } => write!(f, "{file}"),
            FolderElement::Exercise {
                name,
                description: _,
                id: _,
                querypath: _,
                deletion_querypath: _,
            } => write!(f, "Exercise {name}"),
            FolderElement::Opencast {
                name,
                description: _,
                id: _,
                querypath: _,
                deletion_querypath: _,
            } => write!(f, "OpenCast {name}"),
            FolderElement::Viewable {
                name,
                description: _,
                id: _,
                querypath: _,
                deletion_querypath: _,
            } => write!(f, "Folder(-like) {name}"),
        }
    }
}

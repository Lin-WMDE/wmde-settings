// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(feature = "page-about")]
pub mod about;

#[cfg(feature = "page-about")]
pub mod hardware;
#[cfg(feature = "page-about")]
pub mod info;
#[cfg(feature = "page-users")]
pub mod users;

use cosmic_settings_page as page;

#[derive(Default)]
pub struct Page {
    entity: page::Entity,
}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn info(&self) -> page::Info {
        page::Info::new("system", "system-users-symbolic").title(fl!("system"))
    }
}

impl page::AutoBind<crate::pages::Message> for Page {
    fn sub_pages(
        mut page: page::Insert<crate::pages::Message>,
    ) -> page::Insert<crate::pages::Message> {
        #[cfg(feature = "page-users")]
        {
            page = page.sub_page::<users::Page>();
        }

        // WMDE: `about` and `hardware` live here as modules but are registered as
        // top-level pages in `app.rs`, not as sub-pages of this category. A page only
        // gets its own nav-bar entry when it goes through `insert_page`.
        page
    }
}

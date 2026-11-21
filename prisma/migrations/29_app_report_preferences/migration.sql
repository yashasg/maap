CREATE TABLE "app_settings" (
  "website_id" UUID NOT NULL PRIMARY KEY,
  "overview_layout" VARCHAR(20) NOT NULL DEFAULT 'application',
  "color_scheme" VARCHAR(20) NOT NULL DEFAULT 'default',
  "revenue_display_mode" VARCHAR(20) NOT NULL DEFAULT 'none',
  "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT "app_settings_website_id_fkey"
    FOREIGN KEY ("website_id") REFERENCES "website"("website_id")
    ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "app_settings_overview_layout_check"
    CHECK ("overview_layout" IN ('application', 'website')),
  CONSTRAINT "app_settings_color_scheme_check"
    CHECK ("color_scheme" IN ('default', 'vibrant', 'high-contrast', 'emo')),
  CONSTRAINT "app_settings_revenue_display_mode_check"
    CHECK ("revenue_display_mode" IN ('none', 'revenuecat'))
);

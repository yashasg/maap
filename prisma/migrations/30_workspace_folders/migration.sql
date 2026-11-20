CREATE TABLE "workspace_folder" (
  "folder_id" UUID NOT NULL PRIMARY KEY,
  "name" VARCHAR(100) NOT NULL,
  "user_id" UUID,
  "team_id" UUID,
  "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT "workspace_folder_scope_check"
    CHECK (("user_id" IS NOT NULL) <> ("team_id" IS NOT NULL)),
  CONSTRAINT "workspace_folder_user_id_fkey"
    FOREIGN KEY ("user_id") REFERENCES "user"("user_id")
    ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "workspace_folder_team_id_fkey"
    FOREIGN KEY ("team_id") REFERENCES "team"("team_id")
    ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX "workspace_folder_user_id_idx" ON "workspace_folder"("user_id");
CREATE INDEX "workspace_folder_team_id_idx" ON "workspace_folder"("team_id");

CREATE TABLE "website_folder_assignment" (
  "website_id" UUID NOT NULL PRIMARY KEY,
  "folder_id" UUID NOT NULL,
  "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT "website_folder_assignment_website_id_fkey"
    FOREIGN KEY ("website_id") REFERENCES "website"("website_id")
    ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "website_folder_assignment_folder_id_fkey"
    FOREIGN KEY ("folder_id") REFERENCES "workspace_folder"("folder_id")
    ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX "website_folder_assignment_folder_id_idx"
  ON "website_folder_assignment"("folder_id");

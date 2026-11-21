-- CreateTable
CREATE TABLE "notebook" (
    "notebook_id" UUID NOT NULL,
    "website_id" UUID NOT NULL,
    "created_by" UUID,
    "title" VARCHAR(200) NOT NULL,
    "content" JSONB NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "notebook_pkey" PRIMARY KEY ("notebook_id"),
    CONSTRAINT "notebook_title_check" CHECK (length(btrim("title")) BETWEEN 1 AND 200),
    CONSTRAINT "notebook_content_check" CHECK (
        jsonb_typeof("content") = 'object'
        AND "content" @> '{"version": 1}'::jsonb
        AND CASE
            WHEN jsonb_typeof("content" -> 'blocks') = 'array'
            THEN jsonb_array_length("content" -> 'blocks') BETWEEN 1 AND 200
            ELSE false
        END
    )
);

-- CreateIndex
CREATE INDEX "notebook_website_id_updated_at_idx"
    ON "notebook"("website_id", "updated_at" DESC);

-- CreateIndex
CREATE INDEX "notebook_created_by_idx" ON "notebook"("created_by");

-- AddForeignKey
ALTER TABLE "notebook"
    ADD CONSTRAINT "notebook_website_id_fkey"
    FOREIGN KEY ("website_id") REFERENCES "website"("website_id")
    ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "notebook"
    ADD CONSTRAINT "notebook_created_by_fkey"
    FOREIGN KEY ("created_by") REFERENCES "user"("user_id")
    ON DELETE SET NULL ON UPDATE CASCADE;

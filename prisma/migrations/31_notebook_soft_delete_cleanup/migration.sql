DELETE FROM "notebook" AS n
USING "website" AS w
WHERE n."website_id" = w."website_id"
  AND w."deleted_at" IS NOT NULL;

CREATE FUNCTION "delete_notebooks_on_website_soft_delete"()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
  DELETE FROM "notebook" WHERE "website_id" = NEW."website_id";
  RETURN NEW;
END;
$$;

CREATE TRIGGER "website_soft_delete_notebooks"
AFTER UPDATE OF "deleted_at" ON "website"
FOR EACH ROW
WHEN (OLD."deleted_at" IS NULL AND NEW."deleted_at" IS NOT NULL)
EXECUTE FUNCTION "delete_notebooks_on_website_soft_delete"();

-- up
CREATE TABLE IF NOT EXISTS "auth_user" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "username" VARCHAR(150) NOT NULL,
    "email" VARCHAR(254) NOT NULL,
    "password" TEXT NOT NULL,
    "is_active" BOOLEAN NOT NULL,
    "is_staff" BOOLEAN NOT NULL,
    "is_superuser" BOOLEAN NOT NULL,
    "date_joined" TIMESTAMPTZ NOT NULL,
    "last_login" TIMESTAMPTZ
);
CREATE TABLE IF NOT EXISTS "auth_group" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "name" VARCHAR(150) NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS "auth_user_groups" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "user" BIGINT NOT NULL REFERENCES "auth_user"("id") ON DELETE CASCADE,
    "group" BIGINT NOT NULL REFERENCES "auth_group"("id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "auth_permission" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "codename" VARCHAR(255) NOT NULL UNIQUE,
    "name" VARCHAR(255) NOT NULL
);
CREATE TABLE IF NOT EXISTS "auth_group_permissions" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "group" BIGINT NOT NULL REFERENCES "auth_group"("id") ON DELETE CASCADE,
    "permission" BIGINT NOT NULL REFERENCES "auth_permission"("id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "auth_user_permissions" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "user" BIGINT NOT NULL REFERENCES "auth_user"("id") ON DELETE CASCADE,
    "permission" BIGINT NOT NULL REFERENCES "auth_permission"("id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "djangors_admin_log" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "user_id" BIGINT NOT NULL,
    "action_time" TIMESTAMPTZ NOT NULL,
    "app_label" TEXT NOT NULL,
    "model_name" TEXT NOT NULL,
    "object_id" BIGINT NOT NULL,
    "object_repr" TEXT NOT NULL,
    "action_flag" INTEGER NOT NULL,
    "change_message" TEXT NOT NULL,
    "field_diff" TEXT
);
CREATE TABLE IF NOT EXISTS "polls_question" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "question_text" VARCHAR(200) NOT NULL,
    "pub_date" TIMESTAMPTZ NOT NULL
);
CREATE TABLE IF NOT EXISTS "polls_choice" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "choice_text" VARCHAR(200) NOT NULL,
    "votes" INTEGER NOT NULL,
    "question" BIGINT NOT NULL REFERENCES "polls_question"("id") ON DELETE CASCADE
);
-- down
DROP TABLE "auth_user";;
DROP TABLE "auth_group";;
DROP TABLE "auth_user_groups";;
DROP TABLE "auth_permission";;
DROP TABLE "auth_group_permissions";;
DROP TABLE "auth_user_permissions";;
DROP TABLE "djangors_admin_log";;
DROP TABLE "polls_question";;
DROP TABLE "polls_choice";;
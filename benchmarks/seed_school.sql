-- 10.11 reproducible schema/data fixture for djangors_bench.
DROP TABLE IF EXISTS auth_user_permissions, auth_group_permissions, auth_user_groups,
  auth_permission, auth_group, school_enrollment, school_course, school_student, auth_user CASCADE;
CREATE TABLE auth_user (id BIGSERIAL PRIMARY KEY, username VARCHAR(150) NOT NULL UNIQUE,
 email VARCHAR(254) NOT NULL, password TEXT NOT NULL, is_active BOOLEAN NOT NULL,
 is_staff BOOLEAN NOT NULL, is_superuser BOOLEAN NOT NULL, date_joined TIMESTAMPTZ NOT NULL,
 last_login TIMESTAMPTZ);
CREATE TABLE school_student (id BIGSERIAL PRIMARY KEY, first_name VARCHAR(100) NOT NULL,
 last_name VARCHAR(100) NOT NULL, email VARCHAR(254) NOT NULL UNIQUE,
 enrolled_date TIMESTAMPTZ NOT NULL, is_active BOOLEAN NOT NULL);
CREATE TABLE school_course (id BIGSERIAL PRIMARY KEY, code VARCHAR(20) NOT NULL UNIQUE,
 name VARCHAR(200) NOT NULL, credits INTEGER NOT NULL);
CREATE TABLE school_enrollment (id BIGSERIAL PRIMARY KEY, student BIGINT NOT NULL REFERENCES school_student(id),
 course BIGINT NOT NULL REFERENCES school_course(id), enrolled_on TIMESTAMPTZ NOT NULL, grade VARCHAR(5) NOT NULL);
CREATE TABLE auth_permission (id BIGSERIAL PRIMARY KEY, codename VARCHAR(255) NOT NULL UNIQUE, name VARCHAR(255) NOT NULL);
CREATE TABLE auth_group (id BIGSERIAL PRIMARY KEY, name VARCHAR(150) NOT NULL UNIQUE);
CREATE TABLE auth_user_groups (id BIGSERIAL PRIMARY KEY, "user" BIGINT NOT NULL, "group" BIGINT NOT NULL);
CREATE TABLE auth_group_permissions (id BIGSERIAL PRIMARY KEY, "group" BIGINT NOT NULL, permission BIGINT NOT NULL);
CREATE TABLE auth_user_permissions (id BIGSERIAL PRIMARY KEY, "user" BIGINT NOT NULL, permission BIGINT NOT NULL);
INSERT INTO school_student (first_name,last_name,email,enrolled_date,is_active)
SELECT 'Student'||g, CASE WHEN g % 2 = 0 THEN 'Loadtest' ELSE 'Searchable' END,
 'student'||g||'@example.com', TIMESTAMPTZ '2026-01-01' + (g || ' days')::interval, g % 10 <> 0
FROM generate_series(1,5000) AS g;
INSERT INTO school_course (code,name,credits) VALUES ('LOAD101','Load Testing',3);

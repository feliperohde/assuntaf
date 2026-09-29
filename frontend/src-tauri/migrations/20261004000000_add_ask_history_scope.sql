-- Migration: questions can search one project or all projects.
-- An all-projects question is stored in the history of the project that was
-- active when it was asked, with all_projects = 1.

ALTER TABLE ask_history ADD COLUMN all_projects INTEGER NOT NULL DEFAULT 0;

CREATE TABLE `knex_migrations` (`id` integer not null primary key autoincrement, `name` varchar(255));
CREATE TABLE `t_circle` (`id` integer not null primary key, `name` varchar(255) not null);
CREATE TABLE `t_va` (`id` varchar(255), `name` varchar(255) not null);
CREATE TABLE `t_tag` (`id` integer not null primary key, `name` varchar(255) not null);
CREATE TABLE `t_work` (`id` integer not null primary key, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `root_folder` varchar(255) not null, `dir` varchar(255) not null, `title` varchar(255) not null, `circle_id` integer not null, `nsfw` boolean, `release` varchar(255), `dl_count` integer, `price` integer, `rate_count` integer, `rate_average_2dp` float, `rate_count_detail` text, `rank` text, `memo` json, `is_custom_meta` integer default '0');
CREATE TABLE `r_tag_work` (`tag_id` integer, `work_id` bigint);
CREATE TABLE `r_va_work` (`va_id` varchar(255), `work_id` bigint);
CREATE TABLE `t_user` (`name` varchar(255) not null primary key, `password` varchar(255) not null, `group` varchar(255) not null);
CREATE TABLE `t_review` (`user_name` varchar(255) not null, `work_id` bigint not null, `rating` integer, `review_text` varchar(255), `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `progress` varchar(255));
CREATE TABLE `t_play_histroy` (`user_name` varchar(255) not null, `work_id` bigint not null, `created_at` datetime not null default CURRENT_TIMESTAMP, `updated_at` datetime not null default CURRENT_TIMESTAMP, `state` json not null);
CREATE TABLE `t_translate_task` (`id` integer not null primary key);
INSERT INTO knex_migrations (name) VALUES ('20241110142802_custom_work_meta.js');
INSERT INTO t_circle VALUES (11, 'Circle A');
INSERT INTO t_user VALUES ('admin', 'x', 'administrator'), ('guest', 'x', 'user');
INSERT INTO t_work (id, root_folder, dir, title, circle_id, memo) VALUES
  (123456, 'Old', 'RJ123456', 'Scanned', 11, '{"duration":{"01.mp3":60}}'),
  (234567, 'Old', 'RJ234567', 'Not scanned', 11, '{}');
INSERT INTO t_review (user_name, work_id, rating, progress, updated_at) VALUES
  ('admin', 123456, NULL, 'marked', '2024-01-01 00:00:00'),
  ('guest', 234567, 4, NULL, '2024-01-01 00:00:00');
INSERT INTO t_play_histroy (user_name, work_id, updated_at, state) VALUES
  ('admin', 123456, '2024-02-01 00:00:00', '{"queue":[{"title":"01.mp3","subtitle":""}],"index":0,"seconds":30}'),
  ('admin', 234567, '2024-02-01 00:00:00', '{"queue":[{"title":"01.mp3","subtitle":""}],"index":0,"seconds":5}');

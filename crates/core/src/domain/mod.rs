//! Questions that need more than one source, such as how to load filament or how long a print will take. Each
//! question is answered in exactly one place, which also decides which source wins.

pub mod estimates;
pub mod filament;
pub mod filament_match;
pub mod file_catalog;
pub mod job_end;
pub mod light;
pub mod pause;
pub mod prepared_job;
pub mod screen_wake;
pub mod spools;

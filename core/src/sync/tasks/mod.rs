//! The three tasks §21.3 declares, and no fourth.
//!
//! **Neither the clone, the fetch nor the install queue is here** (§21.12, R52): a clone is a
//! user-initiated local git write with a real store and a real disk, and its cost is the
//! per-store cap rather than a rate budget. **Nor is the README asset fetch** (R53): the hosts
//! are arbitrary badge and image services, not the forge, and keying a forge pool by a
//! stranger's `x-ratelimit-resource` would put a wrong value in the row that decides whether the
//! app burns an account's allowance.

pub mod repos;

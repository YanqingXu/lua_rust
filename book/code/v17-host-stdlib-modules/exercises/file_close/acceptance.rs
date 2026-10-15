mod contracts;
mod fixture;
mod model;
mod starter;

#[test]
#[ignore = "TODO: take the File owner and retain the None tombstone"]
fn close_releases_owner_but_keeps_closed_identity() {
    contracts::releases_owner_keeps_identity(starter::close);
}

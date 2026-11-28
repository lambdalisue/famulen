use super::*;
use crate::workers::finishing::tests::{Fake, pinned, rule};

#[tokio::test]
async fn a_step_that_keeps_going_wrong_stops_the_finishing() {
    let mut finishing = Finishing::new(pinned(rule()));
    finishing.turned();
    let hands = Fake {
        lost: true,
        ..Fake::default()
    };
    let seen = Mutex::new(Seen::Working);
    let next = drive(finishing, hands, &seen, Duration::ZERO).await;
    assert!(
        matches!(&next, Next::Stop(why) if why.starts_with("Finishing stopped while checking")),
        "{next:?}"
    );
}

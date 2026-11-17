# spike-net, apple-m1, 2026-11-17

Criterion 7: the netcode spike's four scenes under the clock script and on the server's clock: the correction rate, the largest absorbed difference per kind, rollback cost per tick, and divergence. Threshold: none: recorded (architecture section 22; milestone 1 plan, section 4).

| | |
|---|---|
| CPU | Apple M1 (macos 27.2.1, aarch64) |
| Topology | 4 performance + 4 efficiency cores, as the executor detected them |
| Workers | 4, the spike's |
| Commit | `a539d60461a8` |
| Profile | release |
| Command | `cargo xtask measure --all`, which ran `cargo nextest run --locked --cargo-profile release --package lactose-app --test integration --run-ignored only --retries 0 --no-capture --no-tests=fail -E 'test(=spike::<replay>)'` |
| Load average | 3.86 2.76 2.93 before, 4.96 3.87 3.38 after |
| Replays | `ten_minutes_of_the_dynamic_capsule_with_the_clock_script`, `ten_minutes_of_the_dynamic_capsule_on_the_server_clock`, `ten_minutes_of_the_kinematic_character_with_the_clock_script`, `ten_minutes_of_the_kinematic_character_on_the_server_clock`, `ten_minutes_of_fifty_thrown_bodies_with_the_clock_script`, `ten_minutes_of_fifty_thrown_bodies_on_the_server_clock`, `ten_minutes_of_the_claim_in_contact_with_the_clock_script`, `ten_minutes_of_the_claim_in_contact_on_the_server_clock`, one at a time, in this order |
| Server's regions | every replay's `server_frozen_ticks` is 0: the server held every loaded region `Simulated` on every frame from the load to the end, so no correction is a server-`Frozen` region's (architecture section 9.4); the measurement fails otherwise |
| Prediction bound | at most 64 fixed colliders and proxies together in a prediction world (plan decision 5.5); every replay's `prediction_fixed_and_proxies`, the most its prediction world held after a client frame, is within it, the largest 9 (`claim_in_contact.server_clock`); the measurement fails otherwise |

## Figures

| Figure | Value |
|---|---|
| `dynamic_capsule.clock_script.absorbed.angular` | 3.158084511211832e-15 |
| `dynamic_capsule.clock_script.absorbed.fields` | 351 |
| `dynamic_capsule.clock_script.absorbed.linear` | 2.1588788308657657e-14 |
| `dynamic_capsule.clock_script.absorbed.rotation` | 5.551115123125783e-17 |
| `dynamic_capsule.clock_script.absorbed.translation` | 0.00007760854234017955 |
| `dynamic_capsule.clock_script.carrier_pending_ticks` | 120 |
| `dynamic_capsule.clock_script.corrections_per_minute.mismatch` | 1.0 |
| `dynamic_capsule.clock_script.corrections_per_minute.repaired` | 0.0 |
| `dynamic_capsule.clock_script.corrections_per_minute.substituted` | 0.0 |
| `dynamic_capsule.clock_script.datagrams_dropped` | 506 |
| `dynamic_capsule.clock_script.deepest_correction` | 8 |
| `dynamic_capsule.clock_script.departure_tick` | 488 |
| `dynamic_capsule.clock_script.divergence.entities` | 10 |
| `dynamic_capsule.clock_script.divergence.fields` | 38 |
| `dynamic_capsule.clock_script.divergence.mismatched_entities` | 10 |
| `dynamic_capsule.clock_script.divergence.mismatched_fields` | 38 |
| `dynamic_capsule.clock_script.full_bytes_max` | 3777 |
| `dynamic_capsule.clock_script.hash_agreement.agreed` | 35197 |
| `dynamic_capsule.clock_script.hash_agreement.carrier_agreed` | 35553 |
| `dynamic_capsule.clock_script.hash_agreement.carrier_set_compared` | 117 |
| `dynamic_capsule.clock_script.hash_agreement.compared` | 35553 |
| `dynamic_capsule.clock_script.lead` | 6 |
| `dynamic_capsule.clock_script.mismatches_per_cycle` | [1,9,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0] |
| `dynamic_capsule.clock_script.predicted_keys_at_checks` | [] |
| `dynamic_capsule.clock_script.prediction_fixed_and_proxies` | 5 |
| `dynamic_capsule.clock_script.prefix_equal` | true |
| `dynamic_capsule.clock_script.rate` | 1005 |
| `dynamic_capsule.clock_script.resyncs` | 2 |
| `dynamic_capsule.clock_script.ring.depth` | 60 |
| `dynamic_capsule.clock_script.ring.physics_bytes` | 537670 |
| `dynamic_capsule.clock_script.ring.record_bytes` | 27400 |
| `dynamic_capsule.clock_script.ring.world_bytes` | 2176080 |
| `dynamic_capsule.clock_script.rollback_us_per_tick.median` | 123.2 |
| `dynamic_capsule.clock_script.rollback_us_per_tick.rollbacks` | 10 |
| `dynamic_capsule.clock_script.rollback_us_per_tick.worst` | 154.1 |
| `dynamic_capsule.clock_script.server_frozen_ticks` | 0 |
| `dynamic_capsule.clock_script.substituted_ticks` | 57 |
| `dynamic_capsule.clock_script.thrown_settle_ticks` | 0 |
| `dynamic_capsule.clock_script.ticks` | 36000 |
| `dynamic_capsule.clock_script.tolerance.angular` | 0.02 |
| `dynamic_capsule.clock_script.tolerance.linear` | 0.02 |
| `dynamic_capsule.clock_script.tolerance.rotation` | 0.002 |
| `dynamic_capsule.clock_script.tolerance.translation` | 0.002 |
| `dynamic_capsule.clock_script.unechoed_spawns` | 0 |
| `dynamic_capsule.clock_script.update_bytes.max` | 566 |
| `dynamic_capsule.clock_script.update_bytes.mean` | 456 |
| `dynamic_capsule.clock_script.yaw_error_rad_per_tick.mean` | 1.0850714239990967e-8 |
| `dynamic_capsule.clock_script.yaw_error_rad_per_tick.worst` | 4.16652395328343e-8 |
| `dynamic_capsule.clock_script.wall_s` | 11.0 |
| `dynamic_capsule.server_clock.absorbed.angular` | 1.1744954683779401e-17 |
| `dynamic_capsule.server_clock.absorbed.fields` | 229 |
| `dynamic_capsule.server_clock.absorbed.linear` | 6.661338147750939e-16 |
| `dynamic_capsule.server_clock.absorbed.rotation` | 1.601288942763079e-19 |
| `dynamic_capsule.server_clock.absorbed.translation` | 0.000018390741457263715 |
| `dynamic_capsule.server_clock.carrier_pending_ticks` | 120 |
| `dynamic_capsule.server_clock.corrections_per_minute.mismatch` | 0.1 |
| `dynamic_capsule.server_clock.corrections_per_minute.repaired` | 0.0 |
| `dynamic_capsule.server_clock.corrections_per_minute.substituted` | 0.0 |
| `dynamic_capsule.server_clock.datagrams_dropped` | 557 |
| `dynamic_capsule.server_clock.deepest_correction` | 8 |
| `dynamic_capsule.server_clock.departure_tick` | null |
| `dynamic_capsule.server_clock.divergence.entities` | 1 |
| `dynamic_capsule.server_clock.divergence.fields` | 2 |
| `dynamic_capsule.server_clock.divergence.mismatched_entities` | 1 |
| `dynamic_capsule.server_clock.divergence.mismatched_fields` | 2 |
| `dynamic_capsule.server_clock.full_bytes_max` | 3604 |
| `dynamic_capsule.server_clock.hash_agreement.agreed` | 35701 |
| `dynamic_capsule.server_clock.hash_agreement.carrier_agreed` | 35987 |
| `dynamic_capsule.server_clock.hash_agreement.carrier_set_compared` | 120 |
| `dynamic_capsule.server_clock.hash_agreement.compared` | 35987 |
| `dynamic_capsule.server_clock.lead` | 5 |
| `dynamic_capsule.server_clock.mismatches_per_cycle` | [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0] |
| `dynamic_capsule.server_clock.predicted_keys_at_checks` | [] |
| `dynamic_capsule.server_clock.prediction_fixed_and_proxies` | 5 |
| `dynamic_capsule.server_clock.prefix_equal` | null |
| `dynamic_capsule.server_clock.rate` | 1000 |
| `dynamic_capsule.server_clock.resyncs` | 0 |
| `dynamic_capsule.server_clock.ring.depth` | 60 |
| `dynamic_capsule.server_clock.ring.physics_bytes` | 537690 |
| `dynamic_capsule.server_clock.ring.record_bytes` | 26400 |
| `dynamic_capsule.server_clock.ring.world_bytes` | 2176080 |
| `dynamic_capsule.server_clock.rollback_us_per_tick.median` | 110.8 |
| `dynamic_capsule.server_clock.rollback_us_per_tick.rollbacks` | 1 |
| `dynamic_capsule.server_clock.rollback_us_per_tick.worst` | 110.8 |
| `dynamic_capsule.server_clock.server_frozen_ticks` | 0 |
| `dynamic_capsule.server_clock.substituted_ticks` | 0 |
| `dynamic_capsule.server_clock.thrown_settle_ticks` | 0 |
| `dynamic_capsule.server_clock.ticks` | 36000 |
| `dynamic_capsule.server_clock.tolerance.angular` | 0.02 |
| `dynamic_capsule.server_clock.tolerance.linear` | 0.02 |
| `dynamic_capsule.server_clock.tolerance.rotation` | 0.002 |
| `dynamic_capsule.server_clock.tolerance.translation` | 0.002 |
| `dynamic_capsule.server_clock.unechoed_spawns` | 0 |
| `dynamic_capsule.server_clock.update_bytes.max` | 456 |
| `dynamic_capsule.server_clock.update_bytes.mean` | 456 |
| `dynamic_capsule.server_clock.yaw_error_rad_per_tick.mean` | 1.087695962905596e-8 |
| `dynamic_capsule.server_clock.yaw_error_rad_per_tick.worst` | 4.166523908874509e-8 |
| `dynamic_capsule.server_clock.wall_s` | 11.1 |
| `kinematic_character.clock_script.absorbed.angular` | 0.0 |
| `kinematic_character.clock_script.absorbed.fields` | 0 |
| `kinematic_character.clock_script.absorbed.linear` | 0.0 |
| `kinematic_character.clock_script.absorbed.rotation` | 0.0 |
| `kinematic_character.clock_script.absorbed.translation` | 0.0 |
| `kinematic_character.clock_script.carrier_pending_ticks` | 120 |
| `kinematic_character.clock_script.corrections_per_minute.mismatch` | 0.9 |
| `kinematic_character.clock_script.corrections_per_minute.repaired` | 0.0 |
| `kinematic_character.clock_script.corrections_per_minute.substituted` | 0.0 |
| `kinematic_character.clock_script.datagrams_dropped` | 506 |
| `kinematic_character.clock_script.deepest_correction` | 8 |
| `kinematic_character.clock_script.departure_tick` | 488 |
| `kinematic_character.clock_script.divergence.entities` | 9 |
| `kinematic_character.clock_script.divergence.fields` | 36 |
| `kinematic_character.clock_script.divergence.mismatched_entities` | 9 |
| `kinematic_character.clock_script.divergence.mismatched_fields` | 36 |
| `kinematic_character.clock_script.full_bytes_max` | 3711 |
| `kinematic_character.clock_script.hash_agreement.agreed` | 35520 |
| `kinematic_character.clock_script.hash_agreement.carrier_agreed` | 35556 |
| `kinematic_character.clock_script.hash_agreement.carrier_set_compared` | 117 |
| `kinematic_character.clock_script.hash_agreement.compared` | 35556 |
| `kinematic_character.clock_script.lead` | 6 |
| `kinematic_character.clock_script.mismatches_per_cycle` | [0,9,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0] |
| `kinematic_character.clock_script.predicted_keys_at_checks` | [] |
| `kinematic_character.clock_script.prediction_fixed_and_proxies` | 5 |
| `kinematic_character.clock_script.prefix_equal` | true |
| `kinematic_character.clock_script.rate` | 1005 |
| `kinematic_character.clock_script.resyncs` | 2 |
| `kinematic_character.clock_script.ring.depth` | 60 |
| `kinematic_character.clock_script.ring.physics_bytes` | 464910 |
| `kinematic_character.clock_script.ring.record_bytes` | 27280 |
| `kinematic_character.clock_script.ring.world_bytes` | 2141520 |
| `kinematic_character.clock_script.rollback_us_per_tick.median` | 111.2 |
| `kinematic_character.clock_script.rollback_us_per_tick.rollbacks` | 9 |
| `kinematic_character.clock_script.rollback_us_per_tick.worst` | 169.2 |
| `kinematic_character.clock_script.server_frozen_ticks` | 0 |
| `kinematic_character.clock_script.substituted_ticks` | 57 |
| `kinematic_character.clock_script.thrown_settle_ticks` | 0 |
| `kinematic_character.clock_script.ticks` | 36000 |
| `kinematic_character.clock_script.tolerance.angular` | 0.02 |
| `kinematic_character.clock_script.tolerance.linear` | 0.02 |
| `kinematic_character.clock_script.tolerance.rotation` | 0.002 |
| `kinematic_character.clock_script.tolerance.translation` | 0.002 |
| `kinematic_character.clock_script.unechoed_spawns` | 0 |
| `kinematic_character.clock_script.update_bytes.max` | 541 |
| `kinematic_character.clock_script.update_bytes.mean` | 431 |
| `kinematic_character.clock_script.yaw_error_rad_per_tick.mean` | -0.0 |
| `kinematic_character.clock_script.yaw_error_rad_per_tick.worst` | 0.0 |
| `kinematic_character.clock_script.wall_s` | 10.3 |
| `kinematic_character.server_clock.absorbed.angular` | 0.0 |
| `kinematic_character.server_clock.absorbed.fields` | 0 |
| `kinematic_character.server_clock.absorbed.linear` | 0.0 |
| `kinematic_character.server_clock.absorbed.rotation` | 0.0 |
| `kinematic_character.server_clock.absorbed.translation` | 0.0 |
| `kinematic_character.server_clock.carrier_pending_ticks` | 120 |
| `kinematic_character.server_clock.corrections_per_minute.mismatch` | 0.0 |
| `kinematic_character.server_clock.corrections_per_minute.repaired` | 0.0 |
| `kinematic_character.server_clock.corrections_per_minute.substituted` | 0.0 |
| `kinematic_character.server_clock.datagrams_dropped` | 557 |
| `kinematic_character.server_clock.deepest_correction` | 0 |
| `kinematic_character.server_clock.departure_tick` | null |
| `kinematic_character.server_clock.divergence.entities` | 0 |
| `kinematic_character.server_clock.divergence.fields` | 0 |
| `kinematic_character.server_clock.divergence.mismatched_entities` | 0 |
| `kinematic_character.server_clock.divergence.mismatched_fields` | 0 |
| `kinematic_character.server_clock.full_bytes_max` | 3604 |
| `kinematic_character.server_clock.hash_agreement.agreed` | 35990 |
| `kinematic_character.server_clock.hash_agreement.carrier_agreed` | 35991 |
| `kinematic_character.server_clock.hash_agreement.carrier_set_compared` | 120 |
| `kinematic_character.server_clock.hash_agreement.compared` | 35991 |
| `kinematic_character.server_clock.lead` | 5 |
| `kinematic_character.server_clock.mismatches_per_cycle` | [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0] |
| `kinematic_character.server_clock.predicted_keys_at_checks` | [] |
| `kinematic_character.server_clock.prediction_fixed_and_proxies` | 5 |
| `kinematic_character.server_clock.prefix_equal` | null |
| `kinematic_character.server_clock.rate` | 1000 |
| `kinematic_character.server_clock.resyncs` | 0 |
| `kinematic_character.server_clock.ring.depth` | 60 |
| `kinematic_character.server_clock.ring.physics_bytes` | 464910 |
| `kinematic_character.server_clock.ring.record_bytes` | 26280 |
| `kinematic_character.server_clock.ring.world_bytes` | 2141520 |
| `kinematic_character.server_clock.rollback_us_per_tick.median` | 0.0 |
| `kinematic_character.server_clock.rollback_us_per_tick.rollbacks` | 0 |
| `kinematic_character.server_clock.rollback_us_per_tick.worst` | 0.0 |
| `kinematic_character.server_clock.server_frozen_ticks` | 0 |
| `kinematic_character.server_clock.substituted_ticks` | 0 |
| `kinematic_character.server_clock.thrown_settle_ticks` | 0 |
| `kinematic_character.server_clock.ticks` | 36000 |
| `kinematic_character.server_clock.tolerance.angular` | 0.02 |
| `kinematic_character.server_clock.tolerance.linear` | 0.02 |
| `kinematic_character.server_clock.tolerance.rotation` | 0.002 |
| `kinematic_character.server_clock.tolerance.translation` | 0.002 |
| `kinematic_character.server_clock.unechoed_spawns` | 0 |
| `kinematic_character.server_clock.update_bytes.max` | 431 |
| `kinematic_character.server_clock.update_bytes.mean` | 431 |
| `kinematic_character.server_clock.yaw_error_rad_per_tick.mean` | -0.0 |
| `kinematic_character.server_clock.yaw_error_rad_per_tick.worst` | 0.0 |
| `kinematic_character.server_clock.wall_s` | 9.9 |
| `fifty_thrown_bodies.clock_script.absorbed.angular` | 0.0199995710109393 |
| `fifty_thrown_bodies.clock_script.absorbed.fields` | 1235914 |
| `fifty_thrown_bodies.clock_script.absorbed.linear` | 0.019998927462600014 |
| `fifty_thrown_bodies.clock_script.absorbed.rotation` | 0.001999887229438224 |
| `fifty_thrown_bodies.clock_script.absorbed.translation` | 0.001998241455073213 |
| `fifty_thrown_bodies.clock_script.carrier_pending_ticks` | 120 |
| `fifty_thrown_bodies.clock_script.corrections_per_minute.mismatch` | 772.6 |
| `fifty_thrown_bodies.clock_script.corrections_per_minute.repaired` | 3.0 |
| `fifty_thrown_bodies.clock_script.corrections_per_minute.substituted` | 17.9 |
| `fifty_thrown_bodies.clock_script.datagrams_dropped` | 4254 |
| `fifty_thrown_bodies.clock_script.deepest_correction` | 39 |
| `fifty_thrown_bodies.clock_script.departure_tick` | 488 |
| `fifty_thrown_bodies.clock_script.divergence.entities` | 94034 |
| `fifty_thrown_bodies.clock_script.divergence.fields` | 219239 |
| `fifty_thrown_bodies.clock_script.divergence.mismatched_entities` | 94034 |
| `fifty_thrown_bodies.clock_script.divergence.mismatched_fields` | 219239 |
| `fifty_thrown_bodies.clock_script.full_bytes_max` | 41223 |
| `fifty_thrown_bodies.clock_script.hash_agreement.agreed` | 16022 |
| `fifty_thrown_bodies.clock_script.hash_agreement.carrier_agreed` | 19150 |
| `fifty_thrown_bodies.clock_script.hash_agreement.carrier_set_compared` | 120 |
| `fifty_thrown_bodies.clock_script.hash_agreement.compared` | 19150 |
| `fifty_thrown_bodies.clock_script.lead` | 6 |
| `fifty_thrown_bodies.clock_script.mismatches_per_cycle` | [3,49,5,76,77,76,5,78,77,76,5,79,79,76,5,79,78,75,5,77,78,75,5,80,78,76,4,77,77,77,5,78,75,77,4,78,74,77,5,77,77,77,5,79,92,83,5,77,91,89,4,90,93,88,5,90,94,88,5,95,91,89,5,93,95,88,4,92,94,87,5,95,89,88,5,94,88,90,5,93,93,89,5,89,94,89,5,93,90,90,5,88,88,90,5,92,93,87,5,90,94,86,3,91,90,88,5,93,90,87,5,89,91,89,5,93,92,86,5,90] |
| `fifty_thrown_bodies.clock_script.predicted_keys_at_checks` | [50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50] |
| `fifty_thrown_bodies.clock_script.prediction_fixed_and_proxies` | 5 |
| `fifty_thrown_bodies.clock_script.prefix_equal` | true |
| `fifty_thrown_bodies.clock_script.rate` | 1005 |
| `fifty_thrown_bodies.clock_script.resyncs` | 2 |
| `fifty_thrown_bodies.clock_script.ring.depth` | 60 |
| `fifty_thrown_bodies.clock_script.ring.physics_bytes` | 11088692 |
| `fifty_thrown_bodies.clock_script.ring.record_bytes` | 1340400 |
| `fifty_thrown_bodies.clock_script.ring.world_bytes` | 4436880 |
| `fifty_thrown_bodies.clock_script.rollback_us_per_tick.median` | 431.1 |
| `fifty_thrown_bodies.clock_script.rollback_us_per_tick.rollbacks` | 7935 |
| `fifty_thrown_bodies.clock_script.rollback_us_per_tick.worst` | 551.5 |
| `fifty_thrown_bodies.clock_script.server_frozen_ticks` | 0 |
| `fifty_thrown_bodies.clock_script.substituted_ticks` | 1675 |
| `fifty_thrown_bodies.clock_script.thrown_settle_ticks` | 238 |
| `fifty_thrown_bodies.clock_script.ticks` | 36000 |
| `fifty_thrown_bodies.clock_script.tolerance.angular` | 0.02 |
| `fifty_thrown_bodies.clock_script.tolerance.linear` | 0.02 |
| `fifty_thrown_bodies.clock_script.tolerance.rotation` | 0.002 |
| `fifty_thrown_bodies.clock_script.tolerance.translation` | 0.002 |
| `fifty_thrown_bodies.clock_script.unechoed_spawns` | 1500 |
| `fifty_thrown_bodies.clock_script.update_bytes.max` | 20416 |
| `fifty_thrown_bodies.clock_script.update_bytes.mean` | 12370 |
| `fifty_thrown_bodies.clock_script.yaw_error_rad_per_tick.mean` | -0.0 |
| `fifty_thrown_bodies.clock_script.yaw_error_rad_per_tick.worst` | 0.0 |
| `fifty_thrown_bodies.clock_script.wall_s` | 58.3 |
| `fifty_thrown_bodies.server_clock.absorbed.angular` | 0.01999830672260805 |
| `fifty_thrown_bodies.server_clock.absorbed.fields` | 578384 |
| `fifty_thrown_bodies.server_clock.absorbed.linear` | 0.019991286716801997 |
| `fifty_thrown_bodies.server_clock.absorbed.rotation` | 0.0019995110360070123 |
| `fifty_thrown_bodies.server_clock.absorbed.translation` | 0.0019842330216541804 |
| `fifty_thrown_bodies.server_clock.carrier_pending_ticks` | 120 |
| `fifty_thrown_bodies.server_clock.corrections_per_minute.mismatch` | 326.9 |
| `fifty_thrown_bodies.server_clock.corrections_per_minute.repaired` | 3.0 |
| `fifty_thrown_bodies.server_clock.corrections_per_minute.substituted` | 17.8 |
| `fifty_thrown_bodies.server_clock.datagrams_dropped` | 4849 |
| `fifty_thrown_bodies.server_clock.deepest_correction` | 39 |
| `fifty_thrown_bodies.server_clock.departure_tick` | 13652 |
| `fifty_thrown_bodies.server_clock.divergence.entities` | 27385 |
| `fifty_thrown_bodies.server_clock.divergence.fields` | 63299 |
| `fifty_thrown_bodies.server_clock.divergence.mismatched_entities` | 27385 |
| `fifty_thrown_bodies.server_clock.divergence.mismatched_fields` | 63299 |
| `fifty_thrown_bodies.server_clock.full_bytes_max` | 3604 |
| `fifty_thrown_bodies.server_clock.hash_agreement.agreed` | 25326 |
| `fifty_thrown_bodies.server_clock.hash_agreement.carrier_agreed` | 27979 |
| `fifty_thrown_bodies.server_clock.hash_agreement.carrier_set_compared` | 120 |
| `fifty_thrown_bodies.server_clock.hash_agreement.compared` | 27979 |
| `fifty_thrown_bodies.server_clock.lead` | 5 |
| `fifty_thrown_bodies.server_clock.mismatches_per_cycle` | [3,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,30,2,46,51,0,0,2,52,74,3,14,73,0,0,70,80,35,0,84,45,75,2,86,83,85,5,74,17,82,0,0,5,76,3,48,77,53,4,90,80,88,3,33,44,87,2,34,48,79,2,88,18,88,3,14,5,74,5,90,64,76,2,90,80,83,4,84,93,88,5,53,88,20,5,20] |
| `fifty_thrown_bodies.server_clock.predicted_keys_at_checks` | [50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50,50,50,0,50] |
| `fifty_thrown_bodies.server_clock.prediction_fixed_and_proxies` | 5 |
| `fifty_thrown_bodies.server_clock.prefix_equal` | null |
| `fifty_thrown_bodies.server_clock.rate` | 1000 |
| `fifty_thrown_bodies.server_clock.resyncs` | 0 |
| `fifty_thrown_bodies.server_clock.ring.depth` | 60 |
| `fifty_thrown_bodies.server_clock.ring.physics_bytes` | 11141763 |
| `fifty_thrown_bodies.server_clock.ring.record_bytes` | 1340400 |
| `fifty_thrown_bodies.server_clock.ring.world_bytes` | 4436880 |
| `fifty_thrown_bodies.server_clock.rollback_us_per_tick.median` | 424.8 |
| `fifty_thrown_bodies.server_clock.rollback_us_per_tick.rollbacks` | 3477 |
| `fifty_thrown_bodies.server_clock.rollback_us_per_tick.worst` | 816.7 |
| `fifty_thrown_bodies.server_clock.server_frozen_ticks` | 0 |
| `fifty_thrown_bodies.server_clock.substituted_ticks` | 1621 |
| `fifty_thrown_bodies.server_clock.thrown_settle_ticks` | 238 |
| `fifty_thrown_bodies.server_clock.ticks` | 36000 |
| `fifty_thrown_bodies.server_clock.tolerance.angular` | 0.02 |
| `fifty_thrown_bodies.server_clock.tolerance.linear` | 0.02 |
| `fifty_thrown_bodies.server_clock.tolerance.rotation` | 0.002 |
| `fifty_thrown_bodies.server_clock.tolerance.translation` | 0.002 |
| `fifty_thrown_bodies.server_clock.unechoed_spawns` | 1550 |
| `fifty_thrown_bodies.server_clock.update_bytes.max` | 20306 |
| `fifty_thrown_bodies.server_clock.update_bytes.mean` | 12370 |
| `fifty_thrown_bodies.server_clock.yaw_error_rad_per_tick.mean` | -0.0 |
| `fifty_thrown_bodies.server_clock.yaw_error_rad_per_tick.worst` | 0.0 |
| `fifty_thrown_bodies.server_clock.wall_s` | 42.4 |
| `claim_in_contact.clock_script.absorbed.angular` | 0.019986205331844697 |
| `claim_in_contact.clock_script.absorbed.fields` | 137888 |
| `claim_in_contact.clock_script.absorbed.linear` | 0.019990871056907666 |
| `claim_in_contact.clock_script.absorbed.rotation` | 0.0019401092236883053 |
| `claim_in_contact.clock_script.absorbed.translation` | 0.001999934563348127 |
| `claim_in_contact.clock_script.carrier_pending_ticks` | 120 |
| `claim_in_contact.clock_script.corrections_per_minute.mismatch` | 348.1 |
| `claim_in_contact.clock_script.corrections_per_minute.repaired` | 0.0 |
| `claim_in_contact.clock_script.corrections_per_minute.substituted` | 0.2 |
| `claim_in_contact.clock_script.datagrams_dropped` | 551 |
| `claim_in_contact.clock_script.deepest_correction` | 9 |
| `claim_in_contact.clock_script.departure_tick` | 382 |
| `claim_in_contact.clock_script.divergence.entities` | 6250 |
| `claim_in_contact.clock_script.divergence.fields` | 16068 |
| `claim_in_contact.clock_script.divergence.mismatched_entities` | 6250 |
| `claim_in_contact.clock_script.divergence.mismatched_fields` | 16068 |
| `claim_in_contact.clock_script.full_bytes_max` | 6819 |
| `claim_in_contact.clock_script.hash_agreement.agreed` | 0 |
| `claim_in_contact.clock_script.hash_agreement.carrier_agreed` | 25994 |
| `claim_in_contact.clock_script.hash_agreement.carrier_set_compared` | 0 |
| `claim_in_contact.clock_script.hash_agreement.compared` | 25994 |
| `claim_in_contact.clock_script.lead` | 6 |
| `claim_in_contact.clock_script.mismatches_per_cycle` | [27,32,36,26,26,47,25,24,26,25,26,27,26,37,25,33,25,25,35,25,25,25,43,30,25,26,25,37,26,43,25,26,30,27,25,25,25,25,44,25,25,37,25,25,25,36,26,26,25,28,25,24,34,41,26,27,27,30,27,26,26,25,26,33,26,35,24,26,38,25,29,25,25,35,25,25,36,26,34,26,25,25,34,42,35,26,26,39,25,45,36,25,27,39,26,25,26,24,37,25,25,26,25,25,36,26,36,43,27,27,26,38,26,25,25,26,24,35,26,27] |
| `claim_in_contact.clock_script.predicted_keys_at_checks` | [] |
| `claim_in_contact.clock_script.prediction_fixed_and_proxies` | 9 |
| `claim_in_contact.clock_script.prefix_equal` | true |
| `claim_in_contact.clock_script.rate` | 1005 |
| `claim_in_contact.clock_script.resyncs` | 2 |
| `claim_in_contact.clock_script.ring.depth` | 60 |
| `claim_in_contact.clock_script.ring.physics_bytes` | 1439394 |
| `claim_in_contact.clock_script.ring.record_bytes` | 132000 |
| `claim_in_contact.clock_script.ring.world_bytes` | 4348080 |
| `claim_in_contact.clock_script.rollback_us_per_tick.median` | 137.0 |
| `claim_in_contact.clock_script.rollback_us_per_tick.rollbacks` | 3478 |
| `claim_in_contact.clock_script.rollback_us_per_tick.worst` | 514.7 |
| `claim_in_contact.clock_script.server_frozen_ticks` | 0 |
| `claim_in_contact.clock_script.substituted_ticks` | 57 |
| `claim_in_contact.clock_script.thrown_settle_ticks` | 0 |
| `claim_in_contact.clock_script.ticks` | 36000 |
| `claim_in_contact.clock_script.tolerance.angular` | 0.02 |
| `claim_in_contact.clock_script.tolerance.linear` | 0.02 |
| `claim_in_contact.clock_script.tolerance.rotation` | 0.002 |
| `claim_in_contact.clock_script.tolerance.translation` | 0.002 |
| `claim_in_contact.clock_script.unechoed_spawns` | 0 |
| `claim_in_contact.clock_script.update_bytes.max` | 2170 |
| `claim_in_contact.clock_script.update_bytes.mean` | 1278 |
| `claim_in_contact.clock_script.yaw_error_rad_per_tick.mean` | -0.0 |
| `claim_in_contact.clock_script.yaw_error_rad_per_tick.worst` | 0.0 |
| `claim_in_contact.clock_script.wall_s` | 17.4 |
| `claim_in_contact.server_clock.absorbed.angular` | 0.01999432327647388 |
| `claim_in_contact.server_clock.absorbed.fields` | 137910 |
| `claim_in_contact.server_clock.absorbed.linear` | 0.019962782788855105 |
| `claim_in_contact.server_clock.absorbed.rotation` | 0.00199981176750812 |
| `claim_in_contact.server_clock.absorbed.translation` | 0.001999949448860505 |
| `claim_in_contact.server_clock.carrier_pending_ticks` | 120 |
| `claim_in_contact.server_clock.corrections_per_minute.mismatch` | 338.8 |
| `claim_in_contact.server_clock.corrections_per_minute.repaired` | 0.0 |
| `claim_in_contact.server_clock.corrections_per_minute.substituted` | 0.0 |
| `claim_in_contact.server_clock.datagrams_dropped` | 637 |
| `claim_in_contact.server_clock.deepest_correction` | 8 |
| `claim_in_contact.server_clock.departure_tick` | 382 |
| `claim_in_contact.server_clock.divergence.entities` | 6129 |
| `claim_in_contact.server_clock.divergence.fields` | 15872 |
| `claim_in_contact.server_clock.divergence.mismatched_entities` | 6129 |
| `claim_in_contact.server_clock.divergence.mismatched_fields` | 15872 |
| `claim_in_contact.server_clock.full_bytes_max` | 6534 |
| `claim_in_contact.server_clock.hash_agreement.agreed` | 0 |
| `claim_in_contact.server_clock.hash_agreement.carrier_agreed` | 26453 |
| `claim_in_contact.server_clock.hash_agreement.carrier_set_compared` | 0 |
| `claim_in_contact.server_clock.hash_agreement.compared` | 26453 |
| `claim_in_contact.server_clock.lead` | 5 |
| `claim_in_contact.server_clock.mismatches_per_cycle` | [27,24,24,25,26,26,25,27,25,26,24,36,25,24,26,27,26,26,26,25,25,35,35,26,26,38,26,26,26,26,26,35,25,25,36,26,25,24,25,25,38,25,26,36,27,27,43,25,24,35,26,41,25,35,27,30,24,37,25,37,26,25,47,26,26,27,26,45,25,25,23,42,25,27,25,25,25,26,36,25,24,25,25,28,26,26,24,38,34,23,35,25,24,25,43,26,26,27,25,24,36,25,25,25,44,25,34,25,38,25,32,26,27,24,25,25,25,26,26,26] |
| `claim_in_contact.server_clock.predicted_keys_at_checks` | [] |
| `claim_in_contact.server_clock.prediction_fixed_and_proxies` | 9 |
| `claim_in_contact.server_clock.prefix_equal` | null |
| `claim_in_contact.server_clock.rate` | 1000 |
| `claim_in_contact.server_clock.resyncs` | 0 |
| `claim_in_contact.server_clock.ring.depth` | 60 |
| `claim_in_contact.server_clock.ring.physics_bytes` | 1440378 |
| `claim_in_contact.server_clock.ring.record_bytes` | 132000 |
| `claim_in_contact.server_clock.ring.world_bytes` | 4348080 |
| `claim_in_contact.server_clock.rollback_us_per_tick.median` | 134.9 |
| `claim_in_contact.server_clock.rollback_us_per_tick.rollbacks` | 3388 |
| `claim_in_contact.server_clock.rollback_us_per_tick.worst` | 294.7 |
| `claim_in_contact.server_clock.server_frozen_ticks` | 0 |
| `claim_in_contact.server_clock.substituted_ticks` | 0 |
| `claim_in_contact.server_clock.thrown_settle_ticks` | 0 |
| `claim_in_contact.server_clock.ticks` | 36000 |
| `claim_in_contact.server_clock.tolerance.angular` | 0.02 |
| `claim_in_contact.server_clock.tolerance.linear` | 0.02 |
| `claim_in_contact.server_clock.tolerance.rotation` | 0.002 |
| `claim_in_contact.server_clock.tolerance.translation` | 0.002 |
| `claim_in_contact.server_clock.unechoed_spawns` | 0 |
| `claim_in_contact.server_clock.update_bytes.max` | 2144 |
| `claim_in_contact.server_clock.update_bytes.mean` | 1279 |
| `claim_in_contact.server_clock.yaw_error_rad_per_tick.mean` | -0.0 |
| `claim_in_contact.server_clock.yaw_error_rad_per_tick.worst` | 0.0 |
| `claim_in_contact.server_clock.wall_s` | 17.1 |

## Output

```text

running 1 test
{"scene":"dynamic capsule","clock_script":true,"ticks":36000,"prefix_equal":true,"corrections_per_minute":{"mismatch":1.00,"substituted":0.00,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":351,"translation":7.760854234017955e-5,"rotation":5.551115123125783e-17,"linear":2.1588788308657657e-14,"angular":3.158084511211832e-15},"rollback_us_per_tick":{"median":123.2,"worst":154.1,"rollbacks":10},"divergence":{"entities":10,"fields":38,"mismatched_entities":10,"mismatched_fields":38},"deepest_correction":8,"resyncs":2,"substituted_ticks":57,"unechoed_spawns":0,"hash_agreement":{"compared":35553,"agreed":35197,"carrier_agreed":35553,"carrier_set_compared":117},"carrier_pending_ticks":120,"departure_tick":488,"mismatches_per_cycle":[1, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":1.0850714239990967e-8,"worst":4.16652395328343e-8},"ring":{"depth":60,"world_bytes":2176080,"physics_bytes":537670,"record_bytes":27400},"update_bytes":{"mean":456,"max":566},"full_bytes_max":3777,"datagrams_dropped":506,"rate":1005,"lead":6,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_the_dynamic_capsule_with_the_clock_script ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 11.03s


running 1 test
{"scene":"dynamic capsule","clock_script":false,"ticks":36000,"prefix_equal":null,"corrections_per_minute":{"mismatch":0.10,"substituted":0.00,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":229,"translation":1.8390741457263715e-5,"rotation":1.601288942763079e-19,"linear":6.661338147750939e-16,"angular":1.1744954683779401e-17},"rollback_us_per_tick":{"median":110.8,"worst":110.8,"rollbacks":1},"divergence":{"entities":1,"fields":2,"mismatched_entities":1,"mismatched_fields":2},"deepest_correction":8,"resyncs":0,"substituted_ticks":0,"unechoed_spawns":0,"hash_agreement":{"compared":35987,"agreed":35701,"carrier_agreed":35987,"carrier_set_compared":120},"carrier_pending_ticks":120,"departure_tick":null,"mismatches_per_cycle":[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":1.087695962905596e-8,"worst":4.166523908874509e-8},"ring":{"depth":60,"world_bytes":2176080,"physics_bytes":537690,"record_bytes":26400},"update_bytes":{"mean":456,"max":456},"full_bytes_max":3604,"datagrams_dropped":557,"rate":1000,"lead":5,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_the_dynamic_capsule_on_the_server_clock ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 11.10s


running 1 test
{"scene":"kinematic character","clock_script":true,"ticks":36000,"prefix_equal":true,"corrections_per_minute":{"mismatch":0.90,"substituted":0.00,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":0,"translation":0e0,"rotation":0e0,"linear":0e0,"angular":0e0},"rollback_us_per_tick":{"median":111.2,"worst":169.2,"rollbacks":9},"divergence":{"entities":9,"fields":36,"mismatched_entities":9,"mismatched_fields":36},"deepest_correction":8,"resyncs":2,"substituted_ticks":57,"unechoed_spawns":0,"hash_agreement":{"compared":35556,"agreed":35520,"carrier_agreed":35556,"carrier_set_compared":117},"carrier_pending_ticks":120,"departure_tick":488,"mismatches_per_cycle":[0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":2141520,"physics_bytes":464910,"record_bytes":27280},"update_bytes":{"mean":431,"max":541},"full_bytes_max":3711,"datagrams_dropped":506,"rate":1005,"lead":6,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_the_kinematic_character_with_the_clock_script ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 10.30s


running 1 test
{"scene":"kinematic character","clock_script":false,"ticks":36000,"prefix_equal":null,"corrections_per_minute":{"mismatch":0.00,"substituted":0.00,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":0,"translation":0e0,"rotation":0e0,"linear":0e0,"angular":0e0},"rollback_us_per_tick":{"median":0.0,"worst":0.0,"rollbacks":0},"divergence":{"entities":0,"fields":0,"mismatched_entities":0,"mismatched_fields":0},"deepest_correction":0,"resyncs":0,"substituted_ticks":0,"unechoed_spawns":0,"hash_agreement":{"compared":35991,"agreed":35990,"carrier_agreed":35991,"carrier_set_compared":120},"carrier_pending_ticks":120,"departure_tick":null,"mismatches_per_cycle":[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":2141520,"physics_bytes":464910,"record_bytes":26280},"update_bytes":{"mean":431,"max":431},"full_bytes_max":3604,"datagrams_dropped":557,"rate":1000,"lead":5,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_the_kinematic_character_on_the_server_clock ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 9.88s


running 1 test
{"scene":"fifty thrown bodies","clock_script":true,"ticks":36000,"prefix_equal":true,"corrections_per_minute":{"mismatch":772.60,"substituted":17.90,"repaired":3.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":1235914,"translation":1.998241455073213e-3,"rotation":1.999887229438224e-3,"linear":1.9998927462600014e-2,"angular":1.99995710109393e-2},"rollback_us_per_tick":{"median":431.1,"worst":551.5,"rollbacks":7935},"divergence":{"entities":94034,"fields":219239,"mismatched_entities":94034,"mismatched_fields":219239},"deepest_correction":39,"resyncs":2,"substituted_ticks":1675,"unechoed_spawns":1500,"hash_agreement":{"compared":19150,"agreed":16022,"carrier_agreed":19150,"carrier_set_compared":120},"carrier_pending_ticks":120,"departure_tick":488,"mismatches_per_cycle":[3, 49, 5, 76, 77, 76, 5, 78, 77, 76, 5, 79, 79, 76, 5, 79, 78, 75, 5, 77, 78, 75, 5, 80, 78, 76, 4, 77, 77, 77, 5, 78, 75, 77, 4, 78, 74, 77, 5, 77, 77, 77, 5, 79, 92, 83, 5, 77, 91, 89, 4, 90, 93, 88, 5, 90, 94, 88, 5, 95, 91, 89, 5, 93, 95, 88, 4, 92, 94, 87, 5, 95, 89, 88, 5, 94, 88, 90, 5, 93, 93, 89, 5, 89, 94, 89, 5, 93, 90, 90, 5, 88, 88, 90, 5, 92, 93, 87, 5, 90, 94, 86, 3, 91, 90, 88, 5, 93, 90, 87, 5, 89, 91, 89, 5, 93, 92, 86, 5, 90],"thrown_settle_ticks":238,"predicted_keys_at_checks":[50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":4436880,"physics_bytes":11088692,"record_bytes":1340400},"update_bytes":{"mean":12370,"max":20416},"full_bytes_max":41223,"datagrams_dropped":4254,"rate":1005,"lead":6,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_fifty_thrown_bodies_with_the_clock_script ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 58.31s


running 1 test
{"scene":"fifty thrown bodies","clock_script":false,"ticks":36000,"prefix_equal":null,"corrections_per_minute":{"mismatch":326.90,"substituted":17.80,"repaired":3.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":578384,"translation":1.9842330216541804e-3,"rotation":1.9995110360070123e-3,"linear":1.9991286716801997e-2,"angular":1.999830672260805e-2},"rollback_us_per_tick":{"median":424.8,"worst":816.7,"rollbacks":3477},"divergence":{"entities":27385,"fields":63299,"mismatched_entities":27385,"mismatched_fields":63299},"deepest_correction":39,"resyncs":0,"substituted_ticks":1621,"unechoed_spawns":1550,"hash_agreement":{"compared":27979,"agreed":25326,"carrier_agreed":27979,"carrier_set_compared":120},"carrier_pending_ticks":120,"departure_tick":13652,"mismatches_per_cycle":[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 30, 2, 46, 51, 0, 0, 2, 52, 74, 3, 14, 73, 0, 0, 70, 80, 35, 0, 84, 45, 75, 2, 86, 83, 85, 5, 74, 17, 82, 0, 0, 5, 76, 3, 48, 77, 53, 4, 90, 80, 88, 3, 33, 44, 87, 2, 34, 48, 79, 2, 88, 18, 88, 3, 14, 5, 74, 5, 90, 64, 76, 2, 90, 80, 83, 4, 84, 93, 88, 5, 53, 88, 20, 5, 20],"thrown_settle_ticks":238,"predicted_keys_at_checks":[50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50, 50, 50, 0, 50],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":4436880,"physics_bytes":11141763,"record_bytes":1340400},"update_bytes":{"mean":12370,"max":20306},"full_bytes_max":3604,"datagrams_dropped":4849,"rate":1000,"lead":5,"server_frozen_ticks":0,"prediction_fixed_and_proxies":5}
test spike::ten_minutes_of_fifty_thrown_bodies_on_the_server_clock ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 42.43s


running 1 test
{"scene":"claim in contact","clock_script":true,"ticks":36000,"prefix_equal":true,"corrections_per_minute":{"mismatch":348.10,"substituted":0.20,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":137888,"translation":1.999934563348127e-3,"rotation":1.9401092236883053e-3,"linear":1.9990871056907666e-2,"angular":1.9986205331844697e-2},"rollback_us_per_tick":{"median":137.0,"worst":514.7,"rollbacks":3478},"divergence":{"entities":6250,"fields":16068,"mismatched_entities":6250,"mismatched_fields":16068},"deepest_correction":9,"resyncs":2,"substituted_ticks":57,"unechoed_spawns":0,"hash_agreement":{"compared":25994,"agreed":0,"carrier_agreed":25994,"carrier_set_compared":0},"carrier_pending_ticks":120,"departure_tick":382,"mismatches_per_cycle":[27, 32, 36, 26, 26, 47, 25, 24, 26, 25, 26, 27, 26, 37, 25, 33, 25, 25, 35, 25, 25, 25, 43, 30, 25, 26, 25, 37, 26, 43, 25, 26, 30, 27, 25, 25, 25, 25, 44, 25, 25, 37, 25, 25, 25, 36, 26, 26, 25, 28, 25, 24, 34, 41, 26, 27, 27, 30, 27, 26, 26, 25, 26, 33, 26, 35, 24, 26, 38, 25, 29, 25, 25, 35, 25, 25, 36, 26, 34, 26, 25, 25, 34, 42, 35, 26, 26, 39, 25, 45, 36, 25, 27, 39, 26, 25, 26, 24, 37, 25, 25, 26, 25, 25, 36, 26, 36, 43, 27, 27, 26, 38, 26, 25, 25, 26, 24, 35, 26, 27],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":4348080,"physics_bytes":1439394,"record_bytes":132000},"update_bytes":{"mean":1278,"max":2170},"full_bytes_max":6819,"datagrams_dropped":551,"rate":1005,"lead":6,"server_frozen_ticks":0,"prediction_fixed_and_proxies":9}
test spike::ten_minutes_of_the_claim_in_contact_with_the_clock_script ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 17.38s


running 1 test
{"scene":"claim in contact","clock_script":false,"ticks":36000,"prefix_equal":null,"corrections_per_minute":{"mismatch":338.80,"substituted":0.00,"repaired":0.00},"tolerance":{"translation":2e-3,"rotation":2e-3,"linear":2e-2,"angular":2e-2},"absorbed":{"fields":137910,"translation":1.999949448860505e-3,"rotation":1.99981176750812e-3,"linear":1.9962782788855105e-2,"angular":1.999432327647388e-2},"rollback_us_per_tick":{"median":134.9,"worst":294.7,"rollbacks":3388},"divergence":{"entities":6129,"fields":15872,"mismatched_entities":6129,"mismatched_fields":15872},"deepest_correction":8,"resyncs":0,"substituted_ticks":0,"unechoed_spawns":0,"hash_agreement":{"compared":26453,"agreed":0,"carrier_agreed":26453,"carrier_set_compared":0},"carrier_pending_ticks":120,"departure_tick":382,"mismatches_per_cycle":[27, 24, 24, 25, 26, 26, 25, 27, 25, 26, 24, 36, 25, 24, 26, 27, 26, 26, 26, 25, 25, 35, 35, 26, 26, 38, 26, 26, 26, 26, 26, 35, 25, 25, 36, 26, 25, 24, 25, 25, 38, 25, 26, 36, 27, 27, 43, 25, 24, 35, 26, 41, 25, 35, 27, 30, 24, 37, 25, 37, 26, 25, 47, 26, 26, 27, 26, 45, 25, 25, 23, 42, 25, 27, 25, 25, 25, 26, 36, 25, 24, 25, 25, 28, 26, 26, 24, 38, 34, 23, 35, 25, 24, 25, 43, 26, 26, 27, 25, 24, 36, 25, 25, 25, 44, 25, 34, 25, 38, 25, 32, 26, 27, 24, 25, 25, 25, 26, 26, 26],"thrown_settle_ticks":0,"predicted_keys_at_checks":[],"yaw_error_rad_per_tick":{"mean":-0e0,"worst":0e0},"ring":{"depth":60,"world_bytes":4348080,"physics_bytes":1440378,"record_bytes":132000},"update_bytes":{"mean":1279,"max":2144},"full_bytes_max":6534,"datagrams_dropped":637,"rate":1000,"lead":5,"server_frozen_ticks":0,"prediction_fixed_and_proxies":9}
test spike::ten_minutes_of_the_claim_in_contact_on_the_server_clock ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 17.11s
```

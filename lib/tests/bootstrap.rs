/// The tests in this file are checking that bootstrapping of the database works correctly
/// They are testing edge cases that may accidentally occur with bugs - we wan't to make sure
/// the system can recover in light of these issues.
///
/// We may want to move these tests to another suite, as they aren't testing the statements like
/// the other tests are.
mod helpers;
mod parse;

use helpers::new_ds;
use serial_test::serial;
use surrealdb::err::Error;
use surrealdb::kvs::LockType::Optimistic;
use surrealdb::kvs::Transaction;
use surrealdb::kvs::TransactionType::Write;
use surrealdb::sql::statements::LiveStatement;
use surrealdb::sql::Uuid;

#[tokio::test]
#[serial]
async fn bootstrap_removes_unreachable_nodes() -> Result<(), Error> {
	// Create the datastore
	let dbs = new_ds().await.unwrap();

	let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();
	// Introduce missing nodes (without heartbeats)
	let bad_node = uuid::Uuid::parse_str("9d8e16e4-9f6a-4704-8cf1-7cd55b937c5b").unwrap();
	tx.set_nd(bad_node).await.unwrap();

	// Introduce a valid chain of data to confirm it is not removed from a cleanup
	a_valid_notification(
		&mut tx,
		ValidNotificationState {
			timestamp: None,
			node_id: None,
			live_query_id: None,
			notification_id: None,
			namespace: "testns".to_string(),
			database: "testdb".to_string(),
			table: "testtb".to_string(),
		},
	)
	.await
	.unwrap();

	tx.commit().await.unwrap();

	// Bootstrap
	dbs.bootstrap().await.unwrap();

	// Declare a function that will assert
	async fn try_validate(mut tx: &mut Transaction, bad_node: &uuid::Uuid) -> Result<(), String> {
		let res = tx.scan_nd(1000).await.map_err(|e| e.to_string())?;
		tx.commit().await.map_err(|e| e.to_string())?;
		for node in &res {
			if node.name == bad_node.to_string() {
				return Err(format!("The node name was actually the bad node {:?}", bad_node));
			}
		}
		// {Node generated by bootstrap} + {valid node who's uuid we don't know}
		assert_eq!(res.len(), 2);
		if res.len() != 2 {
			return Err("Expected 2 nodes".to_string());
		}
		Ok(())
	}

	// Verify the incorrect node is deleted, but self and valid still exist
	let res = {
		let mut err = None;
		for _ in 0..5 {
			let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();
			let res = try_validate(&mut tx, &bad_node).await;
			if res.is_ok() {
				return Ok(());
			}
			err = Some(res);
			tokio::time::sleep(std::time::Duration::from_millis(100)).await;
		}
		err.unwrap()
	};
	res.unwrap();
	Ok(())
}

#[tokio::test]
#[serial]
async fn bootstrap_removes_unreachable_node_live_queries() -> Result<(), Error> {
	// Create the datastore
	let dbs = new_ds().await.unwrap();

	// Introduce an invalid node live query
	let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();
	let valid_data = a_valid_notification(
		&mut tx,
		ValidNotificationState {
			timestamp: None,
			node_id: None,
			live_query_id: None,
			notification_id: None,
			namespace: "testns".to_string(),
			database: "testdb".to_string(),
			table: "testtb".to_string(),
		},
	)
	.await
	.unwrap();
	let bad_nd_lq_id = uuid::Uuid::parse_str("67b0f588-2b95-4b6e-87f3-73d0a49034be").unwrap();
	tx.putc_ndlq(
		valid_data.clone().node_id.unwrap().0,
		bad_nd_lq_id,
		&valid_data.namespace,
		&valid_data.database,
		&valid_data.table,
		None,
	)
	.await
	.unwrap();
	tx.commit().await.unwrap();

	// Bootstrap
	dbs.bootstrap().await.unwrap();

	// Verify node live query is deleted
	let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();
	let res = tx.scan_ndlq(valid_data.node_id.as_ref().unwrap(), 1000).await.unwrap();
	tx.commit().await.unwrap();
	assert_eq!(res.len(), 1, "We expect the node to be available");
	let tested_entry = res.get(0).unwrap();
	assert_eq!(tested_entry.lq, valid_data.live_query_id.unwrap());

	Ok(())
}

#[tokio::test]
#[serial]
async fn bootstrap_removes_unreachable_table_live_queries() -> Result<(), Error> {
	// Create the datastore
	let dbs = new_ds().await.unwrap();

	// Introduce an invalid table live query
	let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();
	let valid_data = a_valid_notification(
		&mut tx,
		ValidNotificationState {
			timestamp: None,
			node_id: None,
			live_query_id: None,
			notification_id: None,
			namespace: "testns".to_string(),
			database: "testdb".to_string(),
			table: "testtb".to_string(),
		},
	)
	.await
	.unwrap();
	let bad_tb_lq_id = uuid::Uuid::parse_str("97b8fbe4-a147-4420-95dc-97db3a46c491").unwrap();
	let mut live_stm = LiveStatement::default();
	live_stm.id = bad_tb_lq_id.into();
	tx.putc_tblq(&valid_data.namespace, &valid_data.database, &valid_data.table, live_stm, None)
		.await
		.unwrap();
	tx.commit().await.unwrap();

	// Bootstrap
	dbs.bootstrap().await.unwrap();

	// Verify invalid table live query is deleted
	let mut tx = dbs.transaction(Write, Optimistic).await.unwrap();

	let res = tx
		.scan_tblq(&valid_data.namespace, &valid_data.database, &valid_data.table, 1000)
		.await
		.unwrap();
	tx.commit().await.unwrap();

	assert_eq!(res.len(), 1, "Expected 1 table live query: {:?}", res);
	let tested_entry = res.get(0).unwrap();
	assert_eq!(tested_entry.lq, valid_data.live_query_id.unwrap());
	Ok(())
}

#[tokio::test]
#[serial]
async fn bootstrap_removes_unreachable_live_query_notifications() -> Result<(), Error> {
	Ok(())
}

/// ValidBootstrapState is a representation of a chain of information that bootstrap is concerned
/// with. It is used for two reasons
/// - sometimes we want to detect invalid data that has a valid path (notification without a live query).
/// - sometimes we want to detect existing valid data is not deleted
#[derive(Debug, Clone)]
struct ValidNotificationState {
	pub timestamp: Option<u64>,
	pub node_id: Option<Uuid>,
	pub live_query_id: Option<Uuid>,
	pub notification_id: Option<Uuid>,
	pub namespace: String,
	pub database: String,
	pub table: String,
}

/// Create a chain of valid state that bootstrapping should not remove.
/// As a general rule, there is no need to override the system defaults since this code is to place generic data.
/// If you see these IDs, it is because you captured this entry.
/// So its ok to share ID between tests
async fn a_valid_notification(
	tx: &mut Transaction,
	args: ValidNotificationState,
) -> Result<ValidNotificationState, Error> {
	let now = tx.clock().await;
	let default_node_id =
		Uuid::from(uuid::Uuid::parse_str("123e9d92-c975-4daf-8080-3082e83cfa9b").unwrap());
	let default_lq_id =
		Uuid::from(uuid::Uuid::parse_str("ca02c2d0-31dd-4bf0-ada4-ee02b1191e0a").unwrap());
	let default_not_id =
		Uuid::from(uuid::Uuid::parse_str("c952cf7d-b503-4370-802e-cd2404f2160d").unwrap());
	let entry = ValidNotificationState {
		timestamp: Some(args.timestamp.unwrap_or(now.value)),
		node_id: Some(args.node_id.unwrap_or(default_node_id)),
		live_query_id: Some(args.live_query_id.unwrap_or(default_lq_id)),
		notification_id: Some(args.notification_id.unwrap_or(default_not_id)),
		..args
	};
	let mut live_stm = LiveStatement::default();
	live_stm.id = entry.live_query_id.unwrap();
	live_stm.node = entry.node_id.unwrap();

	// Create heartbeat
	tx.set_hb(entry.timestamp.unwrap().into(), entry.node_id.unwrap().0).await?;
	// Create cluster node entry
	tx.set_nd(entry.node_id.unwrap().0).await?;
	// Create node live query registration
	tx.putc_ndlq(
		entry.node_id.unwrap().0,
		entry.live_query_id.unwrap().0,
		&entry.namespace,
		&entry.database,
		&entry.table,
		None,
	)
	.await?;
	// Create table live query registration
	tx.putc_tblq(&entry.namespace, &entry.database, &entry.table, live_stm, None).await?;
	// TODO Create notification
	// tx.putc_tbnt(
	// ).await?;
	Ok(entry)
}

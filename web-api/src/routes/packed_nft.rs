use std::collections::HashMap;

use kiln_postgres::{Transaction, Validator};
use primitive_types::H160;
use rocket::{get, serde::json::Json};
use rocket_sync_db_pools::diesel;
use serde::Serialize;

use crate::{packed_nft_types::PackedNftTypes, params::Hash160, Error, PgConn};

/// Return the packed list of NFTs this address is eligible to mint
#[get("/address/<address>/nfts")]
pub async fn nfts_by_address(
	conn: PgConn,
	address: Hash160,
) -> Result<Json<PackedNftTypes>, Error> {
	let packed_nft = conn.run(move |c| inner_get_packed_nft(c, address.into())).await?;

	Ok(Json(packed_nft))
}

#[derive(Serialize)]
pub struct AddressNftPair {
	address: H160,
	nft: PackedNftTypes,
}

#[get("/nfts")]
pub async fn list_all_eligible_nft(conn: PgConn) -> Result<Json<Vec<AddressNftPair>>, Error> {
	let issuers = conn.run(move |c| Transaction::list_all_distinct_issuer(c)).await?;

	let pairs = conn
		.run(move |c| {
			issuers
				.into_iter()
				.map(|h| inner_get_packed_nft(c, h).map(|r| AddressNftPair { address: h, nft: r }))
				.collect::<Result<Vec<AddressNftPair>, Error>>()
		})
		.await?;

	Ok(Json(pairs))
}

fn inner_get_packed_nft(
	conn: &diesel::PgConnection,
	address: H160,
) -> Result<PackedNftTypes, Error> {
	let mut packed_nfts = PackedNftTypes::zero();

	// Get the address transaction
	let transactions = Transaction::list_all_from_address(conn, address)?;

	let opt_validator_slashed = Validator::is_validator_slashed(conn, address)?;
	if let Some(slashed) = opt_validator_slashed {
		// is validator
		packed_nfts.set_become_validator();
		// have been slash validator
		if slashed {
			packed_nfts.set_slashed_validator()
		}
	}

	// Do at least 100 transactions
	if transactions.len() >= 100 {
		packed_nfts.set_do_100_tansactions()
	}
	// Do at least 1 transaction
	if !transactions.is_empty() {
		packed_nfts.set_do_one_transaction();
	}

	// Loop over transactions
	// count deployed contracts and calls to smart contracts
	let mut deployed_contracts: usize = 0;
	let mut call_count_by_contract: HashMap<H160, usize> = HashMap::new();
	for t in transactions.into_iter() {
		if t.to().is_none() {
			deployed_contracts += 1;
			continue
		}

		if is_smart_contract_call(&t) {
			let to = t.to().unwrap();
			*call_count_by_contract.entry(to).or_insert(0) += 1
		}
	}

	// deploy 1 contract
	if deployed_contracts > 0 {
		packed_nfts.set_deploy_contract();
	}
	// deploy 10 contracts
	if deployed_contracts >= 10 {
		packed_nfts.set_deploy_10_contract();
	}
	// deploy 50 contracts
	if deployed_contracts >= 50 {
		packed_nfts.set_deploy_50_contract();
	}
	// called to 10 contracts 10 times each
	if call_count_by_contract.into_values().filter(|&v| v >= 10).count() >= 10 {
		packed_nfts.set_do_10_transactions_to_10_contracts()
	}

	Ok(packed_nfts)
}

fn is_smart_contract_call(transaction: &Transaction) -> bool {
	// Non empty transaction input is marker for a call to a smart contract
	!transaction.input().is_empty()
}

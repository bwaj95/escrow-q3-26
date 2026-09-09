use anchor_lang::{
    prelude::Pubkey,
    solana_program::{instruction::Instruction, program_pack::Pack},
    system_program::ID as SYSTEM_PROGRAM_ID,
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::{
    associated_token::{get_associated_token_address, ID as ASSOCIATED_TOKEN_PROGRAM_ID},
    token::spl_token,
};
use litesvm::LiteSVM;
use litesvm_token::{
    spl_token::ID as TOKEN_PROGRAM_ID, CreateAssociatedTokenAccount, CreateMint, MintTo,
};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

const SOL: u64 = 1_000_000_000;
const UNIT: u64 = 1_000_000; // Both mints use six decimals.

const INITIAL_BALANCE: u64 = 100 * UNIT;
const DEPOSIT: u64 = 10 * UNIT;
const RECEIVE: u64 = 20 * UNIT;
const SEED: u64 = 123;

// Retained for the existing make interface.
// Timing is not enforced by the base program.
const EXPIRATION: i64 = 17_780_206_209;

struct Fixture {
    svm: LiteSVM,
    fee_payer: Keypair,
    maker: Keypair,
    taker: Keypair,
    mint_a: Pubkey,
    mint_b: Pubkey,
    maker_ata_a: Pubkey,
    maker_ata_b: Pubkey,
    taker_ata_a: Pubkey,
    taker_ata_b: Pubkey,
    escrow: Pubkey,
    vault: Pubkey,
}

// Shared transaction sender.
//
// The fee payer covers transaction fees. The authority signs the
// instruction as either the maker, taker, or an unauthorized user.
fn send(
    svm: &mut LiteSVM,
    fee_payer: &Keypair,
    authority: &Keypair,
    ix: Instruction,
) -> Result<(), String> {
    // Avoid duplicate transaction signatures when retrying an
    // instruction with the same accounts and data.
    svm.expire_blockhash();

    let message = Message::new(&[ix], Some(&fee_payer.pubkey()));

    let transaction = Transaction::new(&[fee_payer, authority], message, svm.latest_blockhash());

    svm.send_transaction(transaction)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

impl Fixture {
    fn new() -> Self {
        let mut svm = LiteSVM::new();

        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/deploy/escrowq32026.so"
        ));

        svm.add_program(escrowq32026::id(), bytes).unwrap();

        let fee_payer = Keypair::new();
        let maker = Keypair::new();
        let taker = Keypair::new();

        for key in [fee_payer.pubkey(), maker.pubkey(), taker.pubkey()] {
            svm.airdrop(&key, 10 * SOL).unwrap();
        }

        let mint_a = CreateMint::new(&mut svm, &maker)
            .decimals(6)
            .authority(&maker.pubkey())
            .send()
            .unwrap();

        let mint_b = CreateMint::new(&mut svm, &maker)
            .decimals(6)
            .authority(&maker.pubkey())
            .send()
            .unwrap();

        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut svm, &maker, &mint_a)
            .owner(&maker.pubkey())
            .send()
            .unwrap();

        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut svm, &taker, &mint_b)
            .owner(&taker.pubkey())
            .send()
            .unwrap();

        MintTo::new(&mut svm, &maker, &mint_a, &maker_ata_a, INITIAL_BALANCE)
            .send()
            .unwrap();

        // Maker is also mint B's mint authority.
        MintTo::new(&mut svm, &maker, &mint_b, &taker_ata_b, INITIAL_BALANCE)
            .send()
            .unwrap();

        // Derive these without creating them.
        // The take instruction should create them with init_if_needed.
        let maker_ata_b = get_associated_token_address(&maker.pubkey(), &mint_b);

        let taker_ata_a = get_associated_token_address(&taker.pubkey(), &mint_a);

        let (escrow, _) = Pubkey::find_program_address(
            &[b"escrow", maker.pubkey().as_ref(), &SEED.to_le_bytes()],
            &escrowq32026::id(),
        );

        let vault = get_associated_token_address(&escrow, &mint_a);

        Self {
            svm,
            fee_payer,
            maker,
            taker,
            mint_a,
            mint_b,
            maker_ata_a,
            maker_ata_b,
            taker_ata_a,
            taker_ata_b,
            escrow,
            vault,
        }
    }

    fn make_ix(&self, deposit: u64, receive: u64) -> Instruction {
        Instruction {
            program_id: escrowq32026::id(),
            accounts: escrowq32026::accounts::Make {
                maker: self.maker.pubkey(),
                mint_a: self.mint_a,
                mint_b: self.mint_b,
                maker_ata_a: self.maker_ata_a,
                escrow: self.escrow,
                vault: self.vault,
                associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: escrowq32026::instruction::Make {
                seed: SEED,
                deposit,
                receive,
                expiration: EXPIRATION,
            }
            .data(),
        }
    }

    fn take_ix(&self, expected_receive: u64) -> Instruction {
        Instruction {
            program_id: escrowq32026::id(),
            accounts: escrowq32026::accounts::Take {
                taker: self.taker.pubkey(),
                maker: self.maker.pubkey(),
                mint_a: self.mint_a,
                mint_b: self.mint_b,
                taker_ata_a: self.taker_ata_a,
                taker_ata_b: self.taker_ata_b,
                maker_ata_b: self.maker_ata_b,
                escrow: self.escrow,
                vault: self.vault,
                associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: escrowq32026::instruction::Take { expected_receive }.data(),
        }
    }

    fn refund_ix(&self, maker: Pubkey, maker_ata_a: Pubkey) -> Instruction {
        Instruction {
            program_id: escrowq32026::id(),
            accounts: escrowq32026::accounts::Refund {
                maker,
                mint_a: self.mint_a,
                maker_ata_a,
                escrow: self.escrow,
                vault: self.vault,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: escrowq32026::instruction::Refund {}.data(),
        }
    }

    fn update_ix(&self, maker: Pubkey, receive: u64) -> Instruction {
        Instruction {
            program_id: escrowq32026::id(),
            accounts: escrowq32026::accounts::Update {
                maker,
                escrow: self.escrow,
            }
            .to_account_metas(None),
            data: escrowq32026::instruction::Update { receive }.data(),
        }
    }

    fn make(&mut self, deposit: u64, receive: u64) {
        let ix = self.make_ix(deposit, receive);

        send(&mut self.svm, &self.fee_payer, &self.maker, ix).unwrap();
    }

    fn take(&mut self, expected_receive: u64) {
        let ix = self.take_ix(expected_receive);

        send(&mut self.svm, &self.fee_payer, &self.taker, ix).unwrap();
    }

    fn refund(&mut self) {
        let ix = self.refund_ix(self.maker.pubkey(), self.maker_ata_a);

        send(&mut self.svm, &self.fee_payer, &self.maker, ix).unwrap();
    }

    fn update(&mut self, receive: u64) {
        let ix = self.update_ix(self.maker.pubkey(), receive);

        send(&mut self.svm, &self.fee_payer, &self.maker, ix).unwrap();
    }

    fn token_balance(&self, address: &Pubkey) -> u64 {
        let account = self.svm.get_account(address).unwrap();

        spl_token::state::Account::unpack(&account.data)
            .unwrap()
            .amount
    }

    fn sol_balance(&self, address: &Pubkey) -> u64 {
        self.svm
            .get_account(address)
            .map(|account| account.lamports)
            .unwrap_or(0)
    }

    fn state(&self) -> escrowq32026::state::Escrow {
        let account = self.svm.get_account(&self.escrow).unwrap();

        escrowq32026::state::Escrow::try_deserialize(&mut account.data.as_slice()).unwrap()
    }

    fn assert_closed(&self) {
        assert!(self.svm.get_account(&self.escrow).is_none());
        assert!(self.svm.get_account(&self.vault).is_none());
    }
}

#[test]
fn make_stores_offer_and_deposits_tokens() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    let state = f.state();

    let (_, bump) = Pubkey::find_program_address(
        &[b"escrow", f.maker.pubkey().as_ref(), &SEED.to_le_bytes()],
        &escrowq32026::id(),
    );

    assert_eq!(state.seed, SEED);
    assert_eq!(state.maker, f.maker.pubkey());
    assert_eq!(state.mint_a, f.mint_a);
    assert_eq!(state.mint_b, f.mint_b);
    assert_eq!(state.receive, RECEIVE);
    assert_eq!(state.expiration, EXPIRATION);
    assert_eq!(state.bump, bump);

    assert_eq!(
        f.svm.get_account(&f.escrow).unwrap().owner,
        escrowq32026::id()
    );

    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);
    assert_eq!(f.token_balance(&f.vault), DEPOSIT);

    let account = f.svm.get_account(&f.vault).unwrap();
    let vault = spl_token::state::Account::unpack(&account.data).unwrap();

    assert_eq!(account.owner, TOKEN_PROGRAM_ID);
    assert_eq!(vault.owner, f.escrow);
    assert_eq!(vault.mint, f.mint_a);
}

#[test]
fn take_swaps_tokens_and_returns_rent_to_maker() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    // These should be created by take.
    assert!(f.svm.get_account(&f.maker_ata_b).is_none());
    assert!(f.svm.get_account(&f.taker_ata_a).is_none());

    let maker_sol_before = f.sol_balance(&f.maker.pubkey());
    let rent = f.sol_balance(&f.escrow) + f.sol_balance(&f.vault);

    f.take(RECEIVE);

    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);
    assert_eq!(f.token_balance(&f.taker_ata_a), DEPOSIT);
    assert_eq!(f.token_balance(&f.maker_ata_b), RECEIVE);
    assert_eq!(f.token_balance(&f.taker_ata_b), INITIAL_BALANCE - RECEIVE);

    assert_eq!(f.sol_balance(&f.maker.pubkey()), maker_sol_before + rent);

    f.assert_closed();
}

#[test]
fn refund_returns_tokens_and_rent_to_maker() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    let maker_sol_before = f.sol_balance(&f.maker.pubkey());
    let rent = f.sol_balance(&f.escrow) + f.sol_balance(&f.vault);

    f.refund();

    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE);
    assert_eq!(f.token_balance(&f.taker_ata_b), INITIAL_BALANCE);

    assert_eq!(f.sol_balance(&f.maker.pubkey()), maker_sol_before + rent);

    f.assert_closed();
}

#[test]
fn update_changes_price_and_take_uses_new_price() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    let new_receive = 30 * UNIT;
    f.update(new_receive);

    assert_eq!(f.state().receive, new_receive);
    assert_eq!(f.state().maker, f.maker.pubkey());

    // Updating the price must not move the deposited tokens.
    assert_eq!(f.token_balance(&f.vault), DEPOSIT);
    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);

    f.take(new_receive);

    assert_eq!(f.token_balance(&f.maker_ata_b), new_receive);
    assert_eq!(
        f.token_balance(&f.taker_ata_b),
        INITIAL_BALANCE - new_receive
    );
    assert_eq!(f.token_balance(&f.taker_ata_a), DEPOSIT);

    f.assert_closed();
}

#[test]
fn take_rejects_stale_price_without_moving_tokens() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    let new_receive = 30 * UNIT;
    f.update(new_receive);

    // Taker still expects the original price.
    let ix = f.take_ix(RECEIVE);

    let error = send(&mut f.svm, &f.fee_payer, &f.taker, ix).unwrap_err();

    assert!(error.contains("OfferChanged"), "{error}");

    assert_eq!(f.state().receive, new_receive);
    assert_eq!(f.token_balance(&f.vault), DEPOSIT);
    assert_eq!(f.token_balance(&f.taker_ata_b), INITIAL_BALANCE);
    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);

    // ATA creation during account validation is rolled back too.
    assert!(f.svm.get_account(&f.maker_ata_b).is_none());
    assert!(f.svm.get_account(&f.taker_ata_a).is_none());

    // Maker can still recover the deposit.
    f.refund();
    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE);
    f.assert_closed();
}

#[test]
fn take_with_insufficient_tokens_preserves_offer() {
    let mut f = Fixture::new();

    // The taker owns only INITIAL_BALANCE of token B.
    let expensive_offer = INITIAL_BALANCE + UNIT;
    f.make(DEPOSIT, expensive_offer);

    let ix = f.take_ix(expensive_offer);

    let error = send(&mut f.svm, &f.fee_payer, &f.taker, ix).unwrap_err();

    assert!(
        error.to_lowercase().contains("insufficient funds"),
        "{error}"
    );

    assert_eq!(f.state().receive, expensive_offer);
    assert_eq!(f.token_balance(&f.vault), DEPOSIT);
    assert_eq!(f.token_balance(&f.taker_ata_b), INITIAL_BALANCE);
    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);

    assert!(f.svm.get_account(&f.maker_ata_b).is_none());
    assert!(f.svm.get_account(&f.taker_ata_a).is_none());

    f.refund();
    assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE);
    f.assert_closed();
}

#[test]
fn another_user_cannot_update_or_refund() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    // Give the taker a valid token A ATA, so refund fails because
    // they are not the maker, rather than because an ATA is missing.
    let attacker_ata_a = CreateAssociatedTokenAccount::new(&mut f.svm, &f.taker, &f.mint_a)
        .owner(&f.taker.pubkey())
        .send()
        .unwrap();

    let instructions = [
        f.update_ix(f.taker.pubkey(), UNIT),
        f.refund_ix(f.taker.pubkey(), attacker_ata_a),
    ];

    for ix in instructions {
        let error = send(&mut f.svm, &f.fee_payer, &f.taker, ix).unwrap_err();

        assert!(
            error.contains("ConstraintHasOne") || error.contains("ConstraintSeeds"),
            "{error}"
        );

        assert_eq!(f.state().maker, f.maker.pubkey());
        assert_eq!(f.state().receive, RECEIVE);
        assert_eq!(f.token_balance(&f.vault), DEPOSIT);
        assert_eq!(f.token_balance(&attacker_ata_a), 0);
        assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE - DEPOSIT);
    }
}

#[test]
fn make_rejects_zero_amounts() {
    let mut f = Fixture::new();

    for (deposit, receive) in [(0, RECEIVE), (DEPOSIT, 0)] {
        let maker_sol_before = f.sol_balance(&f.maker.pubkey());
        let ix = f.make_ix(deposit, receive);

        let error = send(&mut f.svm, &f.fee_payer, &f.maker, ix).unwrap_err();

        assert!(error.contains("InvalidAmount"), "{error}");

        assert_eq!(f.token_balance(&f.maker_ata_a), INITIAL_BALANCE);
        assert_eq!(f.sol_balance(&f.maker.pubkey()), maker_sol_before);

        // Failed make must not leave initialized accounts behind.
        f.assert_closed();
    }
}

#[test]
fn update_rejects_zero_without_changing_offer() {
    let mut f = Fixture::new();
    f.make(DEPOSIT, RECEIVE);

    let ix = f.update_ix(f.maker.pubkey(), 0);

    let error = send(&mut f.svm, &f.fee_payer, &f.maker, ix).unwrap_err();

    assert!(error.contains("InvalidAmount"), "{error}");

    assert_eq!(f.state().receive, RECEIVE);
    assert_eq!(f.token_balance(&f.vault), DEPOSIT);

    // The original offer remains usable.
    f.take(RECEIVE);

    assert_eq!(f.token_balance(&f.maker_ata_b), RECEIVE);
    assert_eq!(f.token_balance(&f.taker_ata_a), DEPOSIT);

    f.assert_closed();
}

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        close_account, transfer_checked, CloseAccount, Mint, TokenAccount, TokenInterface,
        TransferChecked,
    },
};

use crate::{error::EscrowError, Escrow, ESCROW_SEED};

#[derive(Accounts)]
pub struct Take<'info> {
    #[account(mut)]
    pub taker: Signer<'info>,

    /// CHECK: Bound to escrow.maker by has_one.
    /// Receives the rent returned when the escrow and vault close.
    #[account(mut)]
    pub maker: UncheckedAccount<'info>,

    #[account(
        mint::token_program = token_program
    )]
    pub mint_a: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mint::token_program = token_program
    )]
    pub mint_b: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = taker,
        associated_token::mint = mint_a,
        associated_token::authority = taker,
        associated_token::token_program = token_program
    )]
    pub taker_ata_a: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = mint_b,
        associated_token::authority = taker,
        associated_token::token_program = token_program
    )]
    pub taker_ata_b: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = taker,
        associated_token::mint = mint_b,
        associated_token::authority = maker,
        associated_token::token_program = token_program
    )]
    pub maker_ata_b: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        close = maker,
        has_one = maker,
        has_one = mint_a,
        has_one = mint_b,
        seeds = [
            ESCROW_SEED,
            maker.key().as_ref(),
            escrow.seed.to_le_bytes().as_ref()
        ],
        bump = escrow.bump
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        associated_token::mint = mint_a,
        associated_token::authority = escrow,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> Take<'info> {
    pub fn take(&mut self, expected_receive: u64) -> Result<()> {
        require!(
            self.escrow.receive == expected_receive,
            EscrowError::OfferChanged
        );

        require!(
            self.escrow.receive > 0 && self.vault.amount > 0,
            EscrowError::InvalidAmount
        );

        self.pay_maker()?;
        self.withdraw_and_close_vault()
    }

    // Transfer token B from the taker to the maker.
    pub fn pay_maker(&self) -> Result<()> {
        let cpi_accounts = TransferChecked {
            from: self.taker_ata_b.to_account_info(),
            mint: self.mint_b.to_account_info(),
            to: self.maker_ata_b.to_account_info(),
            authority: self.taker.to_account_info(),
        };

        let cpi_context = CpiContext::new(self.token_program.key(), cpi_accounts);

        transfer_checked(cpi_context, self.escrow.receive, self.mint_b.decimals)
    }

    // Transfer token A to the taker, then close the token vault.
    pub fn withdraw_and_close_vault(&self) -> Result<()> {
        let maker_key = self.maker.key();
        let seed = self.escrow.seed.to_le_bytes();
        let bump = [self.escrow.bump];

        let seeds: &[&[u8]] = &[ESCROW_SEED, maker_key.as_ref(), &seed, &bump];
        let signer_seeds = &[seeds];

        let transfer_accounts = TransferChecked {
            from: self.vault.to_account_info(),
            mint: self.mint_a.to_account_info(),
            to: self.taker_ata_a.to_account_info(),
            authority: self.escrow.to_account_info(),
        };

        let transfer_context =
            CpiContext::new_with_signer(self.token_program.key(), transfer_accounts, signer_seeds);

        transfer_checked(transfer_context, self.vault.amount, self.mint_a.decimals)?;

        let close_accounts = CloseAccount {
            account: self.vault.to_account_info(),
            destination: self.maker.to_account_info(),
            authority: self.escrow.to_account_info(),
        };

        let close_context =
            CpiContext::new_with_signer(self.token_program.key(), close_accounts, signer_seeds);

        close_account(close_context)
    }
}

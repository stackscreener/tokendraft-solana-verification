use anchor_lang::prelude::*;

declare_id!("BSA4cRmwYsbuCcRcmgSrhN51iBJgLBB5QdTK2kpqTDor");

pub const PROGRAM_AUTHORITY: &str = "DCfE4QmioyzLxMFA1i95H2izi78FYE8aD4v2rwavzhiC";

#[derive(Clone, Copy, PartialEq, AnchorSerialize, AnchorDeserialize)]
pub enum TournamentPhase {
    Registration, 
    Playing, 
    Finalized,   
    Cancelled,
}

#[event]
pub struct TournamentCreated {
    pub tournament: Pubkey,
    pub buy_in_amount: u64,
    pub max_players: u8,
    pub match_size: u8,
    pub tournament_prize_percentage: u16,
    pub match_prize_percentage: u16,
    pub operator_fee_percentage: u16,
    pub authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct PlayerBoughtIn {
    pub tournament: Pubkey,
    pub player: Pubkey,
    pub buy_in_amount: u64,
    pub current_players: u8,
    pub timestamp: i64,
}

#[event]
pub struct TournamentStarted {
    pub tournament: Pubkey,
    pub current_players: u8,
    pub payout_percentages: Vec<u16>,
    pub match_payout_percentages: Vec<u16>,
    pub timestamp: i64,
}

#[event]
pub struct TournamentFinalized {
    pub tournament: Pubkey,
    pub winners: Vec<Pubkey>,
    pub total_prize_pool: u128,
    pub timestamp: i64,
}

#[event]
pub struct MatchRewardsDistributed {
    pub tournament: Pubkey,
    pub match_id: u32,
    pub winners: Vec<Pubkey>,
    pub total_match_pool: u128,
    pub timestamp: i64,
}

#[event]
pub struct OperatorFeeWithdrawn {
    pub tournament: Pubkey,
    pub recipient: Pubkey,
    pub amount: u128,
    pub timestamp: i64,
}

#[event]
pub struct TournamentCancelled {
    pub tournament: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct ParticipantRefunded {
    pub tournament: Pubkey,
    pub participant: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub enum Winner {
    Individual(Pubkey),
    Group(Vec<Pubkey>, u8), // players, positions_consumed
}

fn is_participant(tournament_state: &TournamentState, player: &Pubkey) -> bool {
    for participant in tournament_state.participants.iter() {
        if participant == player {
            return true;
        }
    }
    false
}

fn calculate_total_buy_ins(current_players: u8, buy_in_amount: u64) -> Result<u128> {
    let total = current_players as u128 * buy_in_amount as u128;
    require!(
        total / buy_in_amount as u128 == current_players as u128,
        ErrorCode::CalculationOverflow
    );
    Ok(total)
}

fn calculate_percentage_amount(total: u128, percentage: u16) -> Result<u128> {
    let amount = (total * percentage as u128) / 10000;
    require!(
        amount <= total,
        ErrorCode::CalculationOverflow
    );
    Ok(amount)
}

fn transfer_from_escrow<'info>(
    escrow_pda: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    amount: u64,
    tournament_key: Pubkey,
    escrow_bump: u8,
    system_program: &AccountInfo<'info>,
) -> Result<()> {
    let seeds = &[b"escrow", tournament_key.as_ref(), &[escrow_bump]];
    let signer = &[&seeds[..]];
    
    let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
        &escrow_pda.key(),
        &destination.key(),
        amount,
    );
    
    let account_infos = &[escrow_pda.clone(), destination.clone(), system_program.clone()];
    anchor_lang::solana_program::program::invoke_signed(&transfer_ix, account_infos, signer)?;
    Ok(())
}

#[program]
pub mod tournament {
    use super::*;

    pub fn initialize_tournament(
        ctx: Context<InitializeTournament>,
        buy_in_amount: u64,
        max_players: u8,
        match_size: u8,
        tournament_prize_percentage: u16,
        match_prize_percentage: u16,
        operator_fee_percentage: u16,
    ) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(buy_in_amount > 0, ErrorCode::InvalidBuyInAmount);
        require!(max_players >= 2 && max_players <= 100, ErrorCode::InvalidMaxPlayers);
        require!(match_size >= 2 && match_size <= max_players, ErrorCode::InvalidMatchSize);
        require!(tournament_prize_percentage > 0, ErrorCode::InvalidTournamentPrizePercentage);
        require!(operator_fee_percentage > 0 && operator_fee_percentage <= 1500, ErrorCode::InvalidOperatorFeePercentage);
        
        require!(
            tournament_prize_percentage + match_prize_percentage + operator_fee_percentage == 10000,
            ErrorCode::InvalidPercentages
        );
        
        require!(
            tournament_prize_percentage >= 5000, // At least 50% to tournament prizes
            ErrorCode::TournamentPrizeTooLow
        );
        
        require!(
            operator_fee_percentage <= 1500, // Max 15% operator fee
            ErrorCode::OperatorFeeTooHigh
        );
        
        tournament_state.buy_in_amount = buy_in_amount;
        tournament_state.max_players = max_players;
        tournament_state.current_players = 0;
        tournament_state.escrow_bump = ctx.bumps.escrow_pda;
        tournament_state.match_size = match_size;
        tournament_state.phase = TournamentPhase::Registration;
        tournament_state.participants = Vec::new();
        tournament_state.paid_match_ids = Vec::new();
        tournament_state.tournament_prize_percentage = tournament_prize_percentage;
        tournament_state.match_prize_percentage = match_prize_percentage;
        tournament_state.operator_fee_percentage = operator_fee_percentage;
        tournament_state.tournament_payouts = Vec::new();
        tournament_state.match_payout_percentages = Vec::new();
        tournament_state.operator_fee_withdrawn = false;
        tournament_state.refunded_participants = Vec::new();

        tournament_state.authority = ctx.accounts.payer.key();
    
        msg!("Tournament initialized with buy-in: {}, max players: {}, match size: {}", 
             buy_in_amount, max_players, match_size);
        msg!("Prize distribution: Tournament {}%, Match {}%, Operator {}%",
             tournament_prize_percentage / 100, match_prize_percentage / 100, operator_fee_percentage / 100);
        msg!("Escrow PDA: {}", ctx.accounts.escrow_pda.key());
        msg!("Authority: {}", tournament_state.authority);
        
        emit!(TournamentCreated {
            tournament: tournament_state.key(),
            buy_in_amount,
            max_players,
            match_size,
            tournament_prize_percentage,
            match_prize_percentage,
            operator_fee_percentage,
            authority: tournament_state.authority,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn buy_in(ctx: Context<BuyIn>) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Registration, 
            ErrorCode::InvalidPhase
        );
        
        require!(
            tournament_state.current_players < tournament_state.max_players,
            ErrorCode::TournamentFull
        );
        
        require!(
            tournament_state.current_players < u8::MAX,
            ErrorCode::PlayerCountOverflow
        );
        
        for participant in tournament_state.participants.iter() {
            require!(
                *participant != ctx.accounts.player.key(),
                ErrorCode::AlreadyRegistered
            );
        }
        
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.escrow_pda.to_account_info(),
            },
        );
        
        anchor_lang::system_program::transfer(
            cpi_context,
            tournament_state.buy_in_amount,
        )?;
        
        tournament_state.participants.push(ctx.accounts.player.key());
        
        tournament_state.current_players += 1;
        
        msg!("Player {} bought in with {} lamports", 
             ctx.accounts.player.key(), 
             tournament_state.buy_in_amount);
        
        emit!(PlayerBoughtIn {
            tournament: tournament_state.key(),
            player: ctx.accounts.player.key(),
            buy_in_amount: tournament_state.buy_in_amount,
            current_players: tournament_state.current_players,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn start_tournament(
        ctx: Context<StartTournament>,
        payout_percentages: Vec<u16>,
        match_payout_percentages: Vec<u16>,
    ) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Registration,
            ErrorCode::InvalidPhase
        );
        
        require!(
            tournament_state.current_players > 0,
            ErrorCode::NotEnoughPlayers
        );
        
        require!(
            !payout_percentages.is_empty() && payout_percentages.len() <= 20,
            ErrorCode::InvalidPayoutCount
        );
        
        require!(
            payout_percentages.len() <= tournament_state.current_players as usize,
            ErrorCode::TooManyPayoutPositions
        );
        
        let total_percentage: u32 = payout_percentages.iter().map(|&p| p as u32).sum();
        require!(
            total_percentage == 10000,
            ErrorCode::InvalidPercentages
        );
        
        // Validate individual payout percentages
        for (_i, &percentage) in payout_percentages.iter().enumerate() {
            require!(
                percentage > 0,
                ErrorCode::InvalidPayoutPercentage
            );
            require!(
                percentage <= 10000,
                ErrorCode::InvalidPayoutPercentage
            );
        }
        
        // Validate match payout percentages
        require!(
            !match_payout_percentages.is_empty() && match_payout_percentages.len() <= 8,
            ErrorCode::InvalidMatchPayoutCount
        );
        
        let match_total_percentage: u32 = match_payout_percentages.iter().map(|&p| p as u32).sum();
        require!(
            match_total_percentage == 10000,
            ErrorCode::InvalidMatchPayoutPercentages
        );
        
        // Validate individual match payout percentages
        for (_i, &percentage) in match_payout_percentages.iter().enumerate() {
            require!(
                percentage > 0,
                ErrorCode::InvalidMatchPayoutPercentage
            );
            require!(
                percentage <= 10000,
                ErrorCode::InvalidMatchPayoutPercentage
            );
        }
        
        tournament_state.tournament_payouts = payout_percentages;
        tournament_state.match_payout_percentages = match_payout_percentages;
        
        tournament_state.phase = TournamentPhase::Playing;
        
        msg!("Tournament started with {} players and {} payout positions", 
             tournament_state.current_players, tournament_state.tournament_payouts.len());
        msg!("Match payouts: {} positions with {}% total", 
             tournament_state.match_payout_percentages.len(), match_total_percentage / 100);
        
        emit!(TournamentStarted {
            tournament: tournament_state.key(),
            current_players: tournament_state.current_players,
            payout_percentages: tournament_state.tournament_payouts.clone(),
            match_payout_percentages: tournament_state.match_payout_percentages.clone(),
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn finalize_tournament<'a, 'b, 'c, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, FinalizeTournament<'info>>,
        winners: Vec<Winner>
    ) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Playing,
            ErrorCode::InvalidPhase
        );
        
        require!(
            !winners.is_empty(),
            ErrorCode::InvalidWinnerCount
        );
        
        // Validate all winners are participants
        for winner in winners.iter() {
            match winner {
                Winner::Individual(player) => {
                    require!(is_participant(tournament_state, player), ErrorCode::WinnerNotParticipant);
                },
                Winner::Group(players, _) => {
                    for player in players.iter() {
                        require!(is_participant(tournament_state, player), ErrorCode::WinnerNotParticipant);
                    }
                }
            }
        }
        
        let total_buy_ins = calculate_total_buy_ins(tournament_state.current_players, tournament_state.buy_in_amount)?;
        let tournament_pool = calculate_percentage_amount(total_buy_ins, tournament_state.tournament_prize_percentage)?;
        
        let escrow_bump = tournament_state.escrow_bump;
        let tournament_key = tournament_state.key();
        
        // Track position counter for payout percentages
        let mut position_counter = 0;
        let mut total_distributed = 0u128;
        
        for (winner_index, winner) in winners.iter().enumerate() {
            match winner {
                Winner::Individual(player) => {
                    // Single winner - direct payout
                    if position_counter < tournament_state.tournament_payouts.len() {
                        let amount = calculate_percentage_amount(tournament_pool, tournament_state.tournament_payouts[position_counter])?;
                        
                        // Find player account in remaining_accounts
                        let mut player_account_found = false;
                        for account in ctx.remaining_accounts.iter() {
                            if account.key() == *player && !player_account_found {
                                msg!("Transferring {} lamports to tournament winner #{} ({})", 
                                     amount, position_counter + 1, player);
                                
                                transfer_from_escrow(
                                    &ctx.accounts.escrow_pda.to_account_info(),
                                    &account.to_account_info(),
                                    amount as u64,
                                    tournament_key,
                                    escrow_bump,
                                    &ctx.accounts.system_program.to_account_info(),
                                )?;
                                
                                player_account_found = true;
                                break;
                            }
                        }
                        
                        require!(player_account_found, ErrorCode::MissingWinnerAccount);
                        total_distributed += amount;
                    }
                    position_counter += 1;
                },
                Winner::Group(players, positions_consumed) => {
                    // Group of tied winners - combined prize pool
                    require!(
                        !players.is_empty(),
                        ErrorCode::InvalidWinnerCount
                    );
                    
                    require!(
                        *positions_consumed > 0,
                        ErrorCode::InvalidWinnerCount
                    );
                    
                    // Calculate combined prize pool for consumed positions
                    let mut combined_pool_percentage = 0u32;
                    for i in position_counter..(position_counter + *positions_consumed as usize) {
                        if i < tournament_state.tournament_payouts.len() {
                            combined_pool_percentage += tournament_state.tournament_payouts[i] as u32;
                        }
                    }
                    
                    let combined_pool_amount = calculate_percentage_amount(tournament_pool, combined_pool_percentage as u16)?;
                    
                    let payout_per_player = combined_pool_amount / players.len() as u128;
                    
                    require!(
                        payout_per_player * players.len() as u128 <= combined_pool_amount,
                        ErrorCode::CalculationOverflow
                    );
                    
                    let mut remaining_amount = combined_pool_amount;
                    for (player_index, player) in players.iter().enumerate() {
                        let amount_to_transfer = if player_index == players.len() - 1 {
                            remaining_amount
                        } else {
                            payout_per_player
                        };
                        
                        require!(
                            amount_to_transfer > 0,
                            ErrorCode::NoMatchRewards
                        );
                        
                        let mut player_account_found = false;
                        for account in ctx.remaining_accounts.iter() {
                            if account.key() == *player && !player_account_found {
                                msg!("Transferring {} lamports to tied winner group {} player {} ({})", 
                                     amount_to_transfer, winner_index + 1, player_index + 1, player);
                                
                                transfer_from_escrow(
                                    &ctx.accounts.escrow_pda.to_account_info(),
                                    &account.to_account_info(),
                                    amount_to_transfer as u64,
                                    tournament_key,
                                    escrow_bump,
                                    &ctx.accounts.system_program.to_account_info(),
                                )?;
                                
                                player_account_found = true;
                                break;
                            }
                        }
                        
                        require!(player_account_found, ErrorCode::MissingWinnerAccount);
                        
                        remaining_amount -= amount_to_transfer;
                        total_distributed += amount_to_transfer;
                    }
                    
                    // Move position counter forward by consumed positions
                    position_counter += *positions_consumed as usize;
                }
            }
        }
        
        require!(
            total_distributed <= tournament_pool,
            ErrorCode::CalculationOverflow
        );
        
        tournament_state.phase = TournamentPhase::Finalized;
        
        msg!("Tournament finalized, prizes distributed");
        
        let all_winners: Vec<Pubkey> = winners.iter().flat_map(|w| match w {
            Winner::Individual(p) => vec![*p],
            Winner::Group(players, _) => players.clone(),
        }).collect();
        
        emit!(TournamentFinalized {
            tournament: tournament_state.key(),
            winners: all_winners,
            total_prize_pool: total_distributed,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn distribute_match_rewards<'a, 'b, 'c, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, DistributeMatchRewards<'info>>,
        match_id_hash: u32,
        winners: Vec<Winner>,
    ) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Finalized,
            ErrorCode::TournamentNotFinalized
        );
        
        require!(
            !tournament_state.paid_match_ids.contains(&match_id_hash),
            ErrorCode::MatchAlreadyPaid
        );
        
        require!(
            !winners.is_empty(),
            ErrorCode::InvalidWinnerCount
        );
        
        for winner in winners.iter() {
            match winner {
                Winner::Individual(player) => {
                    require!(
                        player != &Pubkey::default(),
                        ErrorCode::InvalidWinner
                    );
                    require!(is_participant(tournament_state, player), ErrorCode::WinnerNotParticipant);
                },
                Winner::Group(players, _) => {
                    for player in players.iter() {
                        require!(
                            player != &Pubkey::default(),
                            ErrorCode::InvalidWinner
                        );
                        require!(is_participant(tournament_state, player), ErrorCode::WinnerNotParticipant);
                    }
                }
            }
        }
        
        let total_buy_ins = calculate_total_buy_ins(tournament_state.current_players, tournament_state.buy_in_amount)?;
        let total_match_pool = calculate_percentage_amount(total_buy_ins, tournament_state.match_prize_percentage)?;
        
        let num_matches = (tournament_state.current_players as u128 + tournament_state.match_size as u128 - 1) / tournament_state.match_size as u128;
        
        require!(
            num_matches > 0,
            ErrorCode::InvalidMatchCount
        );
        
        let match_pool = total_match_pool / num_matches;
        
        require!(
            match_pool > 0,
            ErrorCode::NoMatchRewards
        );
        
        require!(
            match_pool * num_matches <= total_match_pool,
            ErrorCode::CalculationOverflow
        );
        
        let escrow_bump = tournament_state.escrow_bump;
        let tournament_key = tournament_state.key();
        
        let mut position_counter = 0;
        let mut total_distributed = 0u128;
        
        for (winner_index, winner) in winners.iter().enumerate() {
            match winner {
                Winner::Individual(player) => {
                    if position_counter < tournament_state.match_payout_percentages.len() {
                        let amount = calculate_percentage_amount(match_pool, tournament_state.match_payout_percentages[position_counter])?;
                        
                        require!(
                            amount > 0,
                            ErrorCode::NoMatchRewards
                        );
                        
                        let mut winner_account_found = false;
                        for account in ctx.remaining_accounts.iter() {
                            if account.key() == *player && !winner_account_found {
                                msg!("Transferring {} lamports to match winner #{} ({})", 
                                     amount, position_counter + 1, player);
                                
                                transfer_from_escrow(
                                    &ctx.accounts.escrow_pda.to_account_info(),
                                    &account.to_account_info(),
                                    amount as u64,
                                    tournament_key,
                                    escrow_bump,
                                    &ctx.accounts.system_program.to_account_info(),
                                )?;
                                
                                winner_account_found = true;
                                break;
                            }
                        }
                        
                        require!(winner_account_found, ErrorCode::MissingWinnerAccount);
                        total_distributed += amount;
                    }
                    position_counter += 1;
                },
                Winner::Group(players, positions_consumed) => {
                    require!(
                        !players.is_empty(),
                        ErrorCode::InvalidWinnerCount
                    );
                    
                    require!(
                        *positions_consumed > 0,
                        ErrorCode::InvalidWinnerCount
                    );
                    
                    let mut combined_pool_percentage = 0u32;
                    for i in position_counter..(position_counter + *positions_consumed as usize) {
                        if i < tournament_state.match_payout_percentages.len() {
                            combined_pool_percentage += tournament_state.match_payout_percentages[i] as u32;
                        }
                    }
                    
                    let combined_pool_amount = calculate_percentage_amount(match_pool, combined_pool_percentage as u16)?;
                    
                    let payout_per_player = combined_pool_amount / players.len() as u128;
                    
                    require!(
                        payout_per_player * players.len() as u128 <= combined_pool_amount,
                        ErrorCode::CalculationOverflow
                    );
                    
                    let mut remaining_amount = combined_pool_amount;
                    for (player_index, player) in players.iter().enumerate() {
                        let amount_to_transfer = if player_index == players.len() - 1 {
                            remaining_amount
                        } else {
                            payout_per_player
                        };
                        
                        require!(
                            amount_to_transfer > 0,
                            ErrorCode::NoMatchRewards
                        );
                        
                        let mut player_account_found = false;
                        for account in ctx.remaining_accounts.iter() {
                            if account.key() == *player && !player_account_found {
                                msg!("Transferring {} lamports to match tied winner group {} player {} ({})", 
                                     amount_to_transfer, winner_index + 1, player_index + 1, player);
                                
                                transfer_from_escrow(
                                    &ctx.accounts.escrow_pda.to_account_info(),
                                    &account.to_account_info(),
                                    amount_to_transfer as u64,
                                    tournament_key,
                                    escrow_bump,
                                    &ctx.accounts.system_program.to_account_info(),
                                )?;
                                
                                player_account_found = true;
                                break;
                            }
                        }
                        
                        require!(player_account_found, ErrorCode::MissingWinnerAccount);
                        
                        remaining_amount -= amount_to_transfer;
                        total_distributed += amount_to_transfer;
                    }
                    
                    position_counter += *positions_consumed as usize;
                }
            }
        }
        
        require!(
            total_distributed <= match_pool,
            ErrorCode::CalculationOverflow
        );
        
        tournament_state.paid_match_ids.push(match_id_hash);
        
        msg!("Match rewards distributed successfully to {} winners", winners.len());
        
        let all_winners: Vec<Pubkey> = winners.iter().flat_map(|w| match w {
            Winner::Individual(p) => vec![*p],
            Winner::Group(players, _) => players.clone(),
        }).collect();
        
        emit!(MatchRewardsDistributed {
            tournament: tournament_state.key(),
            match_id: match_id_hash,
            winners: all_winners,
            total_match_pool: total_distributed,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn withdraw_operator_fee(ctx: Context<WithdrawOperatorFee>) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Finalized,
            ErrorCode::TournamentNotFinalized
        );
        
        require!(
            !tournament_state.operator_fee_withdrawn,
            ErrorCode::OperatorFeeAlreadyWithdrawn
        );
        
        let total_buy_ins = calculate_total_buy_ins(tournament_state.current_players, tournament_state.buy_in_amount)?;
        let operator_fee = calculate_percentage_amount(total_buy_ins, tournament_state.operator_fee_percentage)?;
        
        let escrow_bump = tournament_state.escrow_bump;
        let tournament_key = tournament_state.key();
        
        transfer_from_escrow(
            &ctx.accounts.escrow_pda.to_account_info(),
            &ctx.accounts.fee_recipient.to_account_info(),
            operator_fee as u64,
            tournament_key,
            escrow_bump,
            &ctx.accounts.system_program.to_account_info(),
        )?;
        
        tournament_state.operator_fee_withdrawn = true;
        
        msg!("Operator fee of {} lamports withdrawn successfully", operator_fee);
        
        emit!(OperatorFeeWithdrawn {
            tournament: tournament_state.key(),
            recipient: ctx.accounts.fee_recipient.key(),
            amount: operator_fee,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn cancel_tournament(ctx: Context<CancelTournament>) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Registration,
            ErrorCode::TournamentAlreadyStarted
        );
        
        // Mark tournament as cancelled
        tournament_state.phase = TournamentPhase::Cancelled;
        
        msg!("Tournament cancelled by authority");
        
        emit!(TournamentCancelled {
            tournament: tournament_state.key(),
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn refund_participant(ctx: Context<RefundParticipant>) -> Result<()> {
        let tournament_state = &mut ctx.accounts.tournament_state;
        
        require!(
            tournament_state.phase == TournamentPhase::Cancelled,
            ErrorCode::TournamentNotCancelled
        );
        
        require!(
            tournament_state.participants.contains(&ctx.accounts.participant.key()),
            ErrorCode::ParticipantNotFound
        );
        
        require!(
            !tournament_state.refunded_participants.contains(&ctx.accounts.participant.key()),
            ErrorCode::ParticipantAlreadyRefunded
        );
        
        let escrow_bump = tournament_state.escrow_bump;
        let tournament_key = tournament_state.key();
        
        // Refund the participant
        transfer_from_escrow(
            &ctx.accounts.escrow_pda.to_account_info(),
            &ctx.accounts.participant.to_account_info(),
            tournament_state.buy_in_amount,
            tournament_key,
            escrow_bump,
            &ctx.accounts.system_program.to_account_info(),
        )?;
        
        // Mark participant as refunded
        tournament_state.refunded_participants.push(ctx.accounts.participant.key());
        
        msg!("Refunded {} lamports to participant {}", 
             tournament_state.buy_in_amount, ctx.accounts.participant.key());
        
        emit!(ParticipantRefunded {
            tournament: tournament_state.key(),
            participant: ctx.accounts.participant.key(),
            amount: tournament_state.buy_in_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

}

#[account]
pub struct TournamentState {
    pub buy_in_amount: u64,    
    pub max_players: u8,       
    pub current_players: u8, 
    pub escrow_bump: u8,       
    pub match_size: u8,     
    pub phase: TournamentPhase, 
    pub participants: Vec<Pubkey>, 
    pub paid_match_ids: Vec<u32>,
    pub tournament_prize_percentage: u16,  
    pub match_prize_percentage: u16,    
    pub operator_fee_percentage: u16,  
    pub tournament_payouts: Vec<u16>, 
    pub match_payout_percentages: Vec<u16>,
    pub operator_fee_withdrawn: bool,  
    pub authority: Pubkey,    
    pub refunded_participants: Vec<Pubkey>,
}

#[derive(Accounts)]
#[instruction(
    buy_in_amount: u64, 
    max_players: u8, 
    match_size: u8,
    tournament_prize_percentage: u16,
    match_prize_percentage: u16,
    operator_fee_percentage: u16,
)]
pub struct InitializeTournament<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + 8 + 1 + 1 + 1 + 1 + 1 + 4 + (32 * 100) + 4 + (4 * 50) + 2 + 2 + 2 + 4 + (2 * 20) + 4 + (2 * 8) + 1 + 32 + 4 + (32 * 100)
    )]
    pub tournament_state: Account<'info, TournamentState>,
   
    #[account(
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump,
    )]
    /// CHECK: This is just a PDA that will hold funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    #[account(
        mut,
        constraint = payer.key().to_string() == PROGRAM_AUTHORITY @ ErrorCode::UnauthorizedAuthority
    )]
    pub payer: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BuyIn<'info> {
    
    #[account(mut)]
    pub tournament_state: Account<'info, TournamentState>,
    
    #[account(
        mut,
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump = tournament_state.escrow_bump,
    )]
    /// CHECK: This is just a PDA that will hold funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    #[account(mut)]
    pub player: Signer<'info>,
    
    #[account(
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct StartTournament<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct FinalizeTournament<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    #[account(
        mut,
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump = tournament_state.escrow_bump,
    )]
    /// CHECK: This is the escrow account that holds funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(match_id_hash: u32, winners: Vec<Pubkey>)]
pub struct DistributeMatchRewards<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    #[account(
        mut,
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump = tournament_state.escrow_bump,
    )]
    /// CHECK: This is the escrow account that holds funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
    
}

#[derive(Accounts)]
pub struct WithdrawOperatorFee<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    #[account(
        mut,
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump = tournament_state.escrow_bump,
    )]
    /// CHECK: This is the escrow account that holds funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    pub authority: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: This is the destination account for the operator fee
    pub fee_recipient: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelTournament<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct RefundParticipant<'info> {
    #[account(
        mut,
        constraint = tournament_state.authority == authority.key() @ ErrorCode::UnauthorizedAuthority
    )]
    pub tournament_state: Account<'info, TournamentState>,
    
    #[account(
        mut,
        seeds = [b"escrow", tournament_state.key().as_ref()],
        bump = tournament_state.escrow_bump,
    )]
    /// CHECK: This is the escrow account that holds funds
    pub escrow_pda: UncheckedAccount<'info>,
    
    pub authority: Signer<'info>,
    
    #[account(mut)]
    /// CHECK: This is the participant account to refund
    pub participant: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid winner account")]
    InvalidWinner,
    #[msg("Invalid tournament phase for this operation")]
    InvalidPhase,
    #[msg("Not enough players to start the tournament")]
    NotEnoughPlayers,
    #[msg("Tournament is full")]
    TournamentFull,
    #[msg("Player has already registered for this tournament")]
    AlreadyRegistered,
    #[msg("Winner is not a tournament participant")]
    WinnerNotParticipant,
    #[msg("Invalid percentages, must sum to 100%")]
    InvalidPercentages,
    #[msg("Invalid match payout percentages")]
    InvalidMatchPayoutPercentages,
    #[msg("Invalid number of payout positions")]
    InvalidPayoutCount,
    #[msg("Invalid number of winners")]
    InvalidWinnerCount,
    #[msg("Cannot have duplicate winners")]
    DuplicateWinner,
    #[msg("Missing winner account in remaining_accounts")]
    MissingWinnerAccount,
    #[msg("Operator fee has already been withdrawn")]
    OperatorFeeAlreadyWithdrawn,
    #[msg("Tournament must be finalized before withdrawing operator fee")]
    TournamentNotFinalized,
    #[msg("Only the authorized authority can perform this action")]
    UnauthorizedAuthority,
    #[msg("Match has already been paid")]
    MatchAlreadyPaid,
    #[msg("Invalid buy-in amount")]
    InvalidBuyInAmount,
    #[msg("Invalid max players")]
    InvalidMaxPlayers,
    #[msg("Invalid match size")]
    InvalidMatchSize,
    #[msg("Invalid tournament prize percentage")]
    InvalidTournamentPrizePercentage,
    #[msg("Invalid match prize percentage")]
    InvalidMatchPrizePercentage,
    #[msg("Invalid operator fee percentage")]
    InvalidOperatorFeePercentage,
    #[msg("Player count overflow")]
    PlayerCountOverflow,
    #[msg("Too many payout positions")]
    TooManyPayoutPositions,
    #[msg("Invalid payout percentage")]
    InvalidPayoutPercentage,
    #[msg("Invalid match count")]
    InvalidMatchCount,
    #[msg("No match rewards to distribute")]
    NoMatchRewards,
    #[msg("Invalid match payout count")]
    InvalidMatchPayoutCount,
    #[msg("Invalid match payout percentage")]
    InvalidMatchPayoutPercentage,
    #[msg("Too many winners")]
    TooManyWinners,
    #[msg("Tournament prize too low")]
    TournamentPrizeTooLow,
    #[msg("Operator fee too high")]
    OperatorFeeTooHigh,
    #[msg("Calculation overflow")]
    CalculationOverflow,
    #[msg("Tournament already started")]
    TournamentAlreadyStarted,
    #[msg("Tournament not cancelled")]
    TournamentNotCancelled,
    #[msg("Participant not found")]
    ParticipantNotFound,
    #[msg("Participant already refunded")]
    ParticipantAlreadyRefunded,
}
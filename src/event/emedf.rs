#![allow(unused)]

use crate::{
    lua_emevd_commands, event::emevd_utils::execute_emevd_instruction, event::emk_system::EmkInstruction
};

pub fn initialize_common_event(slot_id: i32, event_id: u32, event_args: &[i32]) {
    // Slot should generally be unique for dynamically generated events.
    // There is a condition where the event runs if eventId == -1 || (eventSlot < 0 && (eventFlagId / 1000) % 10 != 5),
    // where eventFlagId is eventId if slot <= 0, but it's not great to rely on.
    // Otherwise, the condition it uses is that eventId and eventFlagId (i.e. eventSlot) must be distinct.
    let mut args = vec![slot_id, event_id as i32];
    args.extend_from_slice(event_args);
    // Length is required for args to get passed in and copied
    let instruction = EmkInstruction::new_with_length(2000, 0, args.len() * 4);
    unsafe { execute_emevd_instruction(instruction, args.as_ptr() as *const u8); }
}

lua_emevd_commands! {
    // fn register_emevd_commands(lua: &Lua, table: &LuaTable) -> LuaResult<()>;

    // System
    struct SaveRequest(2000, 5) {
        dummy: u8 = 0,
    }
    struct StartPS5Activity(2000, 7) {
        activity_id: i32 = 0,
    }
    struct EndPS5Activity(2000, 8) {
        activity_id: i32 = 0,
    }

    // Timer
    struct SetCurrentTime(2001, 4) {
        hours: u8 = 0,
        minutes: u8 = 0,
        seconds: u8 = 0,
        fade_transition: bool = false,
        should_wait_for_completion: bool = false,
        show_clock: bool = false,
        clock_startup_delay_s: f32 = 0f32,
        clock_move_time_s: f32 = 0f32,
        clock_finish_delay_s: f32 = 0f32,
    }
    struct FreezeTime(2001, 5) {
        should_freeze: bool = false,
    }

    // Cutscene
    struct PlayCutsceneToAll(2002, 1) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
    }
    struct PlayCutsceneToPlayer(2002, 3) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
        player_entity_id: u32 = 0,
    }
    struct PlayCutsceneToPlayerWithWeatherAndTime(2002, 10) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
        player_entity_id: u32 = 0,
        should_change_weather: bool = false,
        weather: i8 = 0, // enum: Weather
        weather_lifespan_seconds: f32 = 0f32,
        should_change_time: bool = false,
        hours: u8 = 0,
        minutes: u8 = 0,
        seconds: u8 = 0,
    }
    struct PlayCutsceneToPlayerAndWarp(2002, 11) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
        area_entity_id: u32 = 0,
        map_id: i32 = 0,
        player_entity_id: u32 = 0,
        unknown14: i32 = 0,
        unknown18: bool = false,
    }
    struct PlayCutsceneToPlayerAndWarpWithWeatherAndTime(2002, 12) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
        area_entity_id: u32 = 0,
        map_id: i32 = 0,
        player_entity_id: u32 = 0,
        unknown14: i32 = 0,
        unknown18: bool = false,
        should_change_weather: bool = false,
        weather: i8 = 0, // enum: Weather
        weather_lifespan_seconds: f32 = 0f32,
        should_change_time: bool = false,
        hours: u8 = 0,
        minutes: u8 = 0,
        seconds: u8 = 0,
    }
    struct PlayCutsceneToPlayerAndWarpWithStablePositionUpdate(2002, 13) {
        cutscene_id: i32 = 0,
        playback_method: u32 = 0, // enum: CutscenePlayMode
        area_entity_id: u32 = 0,
        map_id: i32 = 0,
        player_entity_id: u32 = 0,
        unknown14: i32 = 0,
        unknown18: bool = false,
        should_update_stable_position: bool = false,
    }

    // Event
    struct AwardItemLot(2003, 4) {
        item_lot_id: i32 = 0,
    }
    struct ShootBullet(2003, 5) {
        bullet_team_entity_id: u32 = 0,
        bullet_producer_entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        behavior_id: i32 = 0,
        firing_angle_x: i32 = 0,
        firing_angle_y: i32 = 0,
        firing_angle_z: i32 = 0,
    }
    struct InvertEventFlag(2003, 9) {
        event_flag_id: u32 = 0,
    }
    struct DisplayBossHealthBar(2003, 11) {
        disabled_enabled: i8 = 0, // enum: DisabledEnabled
        entity_id: u32 = 0,
        slot_number: i16 = 0,
        name_id: i32 = 0,
    }
    struct HandleBossDefeatAndDisplayBanner(2003, 12) {
        entity_id: u32 = 0,
        banner_type: u8 = 8, // enum: TextBannerType
    }
    struct ModifyNavimeshConnectionBitflag(2003, 13) {
        navimesh_entity_id: u32 = 0,
        navimesh_type: u32 = 128, // enum: NavimeshType
        type_bit_operation: u8 = 1, // enum: BitopType
    }
    struct WarpPlayer(2003, 14) {
        area_id: u8 = 0,
        block_id: u8 = 0,
        region_id: u8 = 0,
        index_id: u8 = 0,
        initial_area_entity_id: u32 = 0,
        subarea_name_popup_message_id: i32 = 0,
    }
    struct HandleMinibossDefeat(2003, 15) {
        entity_id: u32 = 0,
    }
    struct TriggerMultiplayerEvent(2003, 16) {
        multiplayer_event_id: u32 = 0,
    }
    struct RandomlySetEventFlagInRange(2003, 17) {
        event_flag_id_min: u32 = 0,
        event_flag_id_max: u32 = 0,
        flag_state: u8 = 1, // enum: OnOff
    }
    struct ForceAnimationPlayback(2003, 18) {
        entity_id: u32 = 0,
        animation_id: i32 = -1,
        should_loop: bool = false,
        should_wait_for_completion: bool = false,
        ignore_wait_for_transition: bool = false,
        comparison_type: i8 = 0, // enum: ComparisonType
        number_of_target_characters: f32 = 1f32,
    }
    struct IncrementGameCycle(2003, 21) {
        dummy: u8 = 0,
    }
    struct BatchSetEventFlags(2003, 22) {
        event_flag_id_start: u32 = 0,
        event_flag_id_end: u32 = 0,
        flag_state: u8 = 1, // enum: OnOff
    }
    struct SetPlayerRespawnPoint(2003, 23) {
        respawn_area_entity_id: u32 = 0,
    }
    struct RemoveItemFromPlayer(2003, 24) {
        item_type: u8 = 0, // enum: ItemType
        item_id: i32 = 0,
        number: i32 = 0,
    }
    struct PlaceNPCSummonSign(2003, 25) {
        sign_type: i32 = 0, // enum: SummonSignType
        summoned_npc_entity_id: u32 = 0,
        spawn_area_entity_id: u32 = 0,
        summon_event_flag_id: u32 = 0,
        dismissal_event_flag_id: u32 = 0,
        unknown: bool = false,
    }
    struct AwardAchievement(2003, 28) {
        achievement_id: i32 = 0,
    }
    struct IncrementEventValue(2003, 31) {
        base_event_flag_id: u32 = 0,
        number_of_used_flag_bits: u32 = 1,
        maximum_allowed_value: u32 = 0,
    }
    struct ClearEventValue(2003, 32) {
        base_event_flag_id: u32 = 0,
        number_of_used_flag_bits: u32 = 1,
    }
    struct SetSnugglyNextTrade(2003, 33) {
        event_flag_id: u32 = 0,
    }
    struct SpawnSnugglyItem(2003, 34) {
        item_lot_id: i32 = 0,
        placement_area_entity_id: u32 = 0,
        event_flag_id: u32 = 0,
        hit_entity_id: u32 = 0,
    }
    struct MoveBloodstainAndDroppedItems(2003, 35) {
        source_area_entity_id: u32 = 0,
        destination_area_entity_id: u32 = 0,
    }
    struct AwardItemsIncludingClients(2003, 36) {
        item_lot_id: i32 = 0,
    }
    struct EventValueOperation(2003, 41) {
        base_event_flag_id: u32 = 0,
        number_of_used_flag_bits: u32 = 1,
        operand: i32 = 0,
        base_event_flag_id_operand: u32 = 0,
        number_of_used_event_flag_bits_operand: u32 = 1,
        calculation_type: i8 = 0, // enum: CalculationType
    }
    struct StoreItemAmountHeldInEventValue(2003, 42) {
        item_type: u8 = 0, // enum: ItemType
        item_id: i32 = 0,
        base_event_flag_id: u32 = 0,
        number_of_used_flag_bits: u32 = 1,
    }
    struct DirectlyGivePlayerItem(2003, 43) {
        item_type: u8 = 0, // enum: ItemType
        item_id: i32 = 0,
        base_event_flag_id: u32 = 0,
        number_of_used_flag_bits: u32 = 1,
    }
    struct TriggerAISound(2003, 52) {
        ai_sound_param_id: i32 = 0,
        entity_id: u32 = 0,
        origin_entity_type: u8 = 0, // enum: TargetEntityType
    }
    struct InvokeEnemyGenerator(2003, 54) {
        generator_entity_id: u32 = 0,
    }
    struct BatchSetNetworkConnectedEventFlags(2003, 63) {
        event_flag_id_start: u32 = 0,
        event_flag_id_end: u32 = 0,
        flag_state: u8 = 1, // enum: OnOff
    }
    struct SetOmissionModeCounts(2003, 64) {
        level_1_count: i32 = 0,
        level_2_count: i32 = 0,
    }
    struct ResetOmissionModeCountsToDefault(2003, 65) {}
    struct SetEventFlag(2003, 66) {
        target_event_flag_type: u8 = 0, // enum: TargetEventFlagType
        target_event_flag_id: u32 = 0,
        desired_flag_state: u8 = 1, // enum: OnOff
    }
    struct ChangeWeather(2003, 68) {
        weather: i8 = 0, // enum: Weather
        lifespan_seconds: f32 = 0f32,
        should_switch_immediately: bool = false,
    }
    struct SetNetworkConnectedEventFlag(2003, 69) {
        target_event_flag_type: u8 = 0, // enum: TargetEventFlagType
        target_event_flag_id: u32 = 0,
        desired_flag_state: u8 = 1, // enum: OnOff
    }
    struct TriggerAreaReload(2003, 70) {
        ignore_is_my_world_check: bool = false,
    }
    struct AwardGesture(2003, 71) {
        gesture_param_id: i32 = 0,
    }
    struct ReduceBloodstainSouls(2003, 72) {
        decrease_rate: f32 = 0f32,
        soul_amount_save_slot_id: i32 = 0,
    }
    struct IncreaseEnemySoulDropAmount(2003, 73) {
        entity_id: u32 = 0,
        fixed_increase_amount: i32 = 0,
        soul_amount_load_slot_id: i32 = 0,
    }
    struct IssueEndOfPseudoMultiplayerNotification(2003, 74) {
        success: bool = false,
    }
    struct UseFarViewCamera(2003, 75) {
        far_view_id: u32 = 0,
        asset_entity_id: u32 = 0,
        dummypoly_id: i32 = 0,
    }
    struct SetPlayerPositionDisplay(2003, 76) {
        state: u8 = 0, // enum: DisabledEnabled
        aboveground: bool = false,
        area_id: u8 = 0,
        block_id: u8 = 0,
        region_id: u8 = 0,
        index_id: u8 = 0,
        x: f32 = 0f32,
        y: f32 = 0f32,
        z: f32 = 0f32,
    }
    struct SetPsuedoMultiplayerReturnPosition(2003, 77) {
        area_entity_id: u32 = 0,
    }
    struct OpenWorldMapPoint(2003, 78) {
        world_map_point_param_id: i32 = 0,
        distance: f32 = 0f32,
    }
    struct SendNPCSummonHome(2003, 79) {
        npc_entity_id: u32 = 0,
    }
    struct RemoveGesture(2003, 81) {
        gesture_param_id: i32 = 0,
    }
    struct DeleteNPCSummonSign(2003, 82) {
        npc_entity_id: u32 = 0,
    }

    // Character
    struct SetCharacterTeamType(2004, 2) {
        entity_id: u32 = 0,
        team_type: u8 = 0, // enum: TeamType
    }
    struct ForceCharacterDeath(2004, 4) {
        entity_id: u32 = 0,
        should_receive_runes: bool = true,
    }
    struct EzstateInstructionRequest(2004, 6) {
        entity_id: u32 = 0,
        command: i32 = 0,
        slot: u8 = 0,
    }
    struct CreateBulletOwner(2004, 7) {
        entity_id: u32 = 0,
    }
    struct SetSpEffect(2004, 8) {
        entity_id: u32 = 0,
        sp_effect_id: i32 = 0,
    }
    struct SetCharacterEventTarget(2004, 11) {
        entity_id: u32 = 0,
        entity_id_2: u32 = 0,
    }
    struct SetCharacterHome(2004, 13) {
        entity_id: u32 = 0,
        area_entity_id: u32 = 0,
    }
    struct RotateCharacter(2004, 14) {
        entity_id: u32 = 0,
        relative_entity_id: u32 = 0,
        animation_id: i32 = -1,
        should_wait_for_completion: bool = false,
    }
    struct ClearCharactersAITarget(2004, 16) {
        entity_id: u32 = 0,
    }
    struct RequestCharacterAICommand(2004, 17) {
        entity_id: u32 = 0,
        command_id: i32 = 0,
        slot_number: u8 = 0,
    }
    struct SetEventPoint(2004, 18) {
        entity_id: u32 = 0,
        relative_entity_id: u32 = 0,
        reaction_distance: f32 = 0f32,
    }
    struct SetCharacterAIId(2004, 19) {
        entity_id: u32 = 0,
        ai_id: i32 = 0,
    }
    struct RequestCharacterAIRePlan(2004, 20) {
        entity_id: u32 = 0,
    }
    struct ClearSpEffect(2004, 21) {
        entity_id: u32 = 0,
        sp_effect_id: i32 = 0,
    }
    struct CreateNPCPart(2004, 22) {
        entity_id: u32 = 0,
        npc_part_id: i16 = 0,
        npc_part_group_idx: i16 = 1, // enum: NPCPartType
        npc_part_hp: i32 = 0,
        damage_correction: f32 = 1f32,
        body_damage_compensation: f32 = 1f32,
        is_invincible: bool = false,
        start_in_stopped_state: bool = false,
    }
    struct SetNPCPartHP(2004, 23) {
        entity_id: u32 = 0,
        npc_part_id: i32 = 0,
        desired_hp: i32 = 0,
        should_allow_hp_above_max_hp: bool = false,
    }
    struct SetNPCPartSEAndSFX(2004, 24) {
        entity_id: u32 = 0,
        npc_part_id: i32 = 0,
        defense_material_se_id: i32 = -1,
        defense_material_sfx_id: i32 = -1,
        unknowna: i32 = -1,
        unknownb: i32 = -1,
        unknownc: i32 = -1,
    }
    struct SetNPCPartBulletDamageMultiplier(2004, 25) {
        entity_id: u32 = 0,
        npc_part_id: i32 = 0,
        bullet_damage_multiplier: f32 = 1f32,
    }
    struct ChangeCharacterDispmask(2004, 26) {
        entity_id: u32 = 0,
        bit_number: u8 = 0,
        switch_type: u8 = 1, // enum: OnOffChange
    }
    struct ChangeCharacterHitmask(2004, 27) {
        character_entity_id: u32 = 0,
        bit_number: u8 = 0,
        switch_type: u8 = 1, // enum: OnOffChange
    }
    struct SetNetworkUpdateAuthority(2004, 28) {
        entity_id: u32 = 0,
        authority_level: i32 = 0, // enum: AuthorityLevel
    }
    struct SetCharacterBackreadState(2004, 29) {
        entity_id: u32 = 0,
        removed: bool = true,
    }
    struct SetCharacterMaphit(2004, 31) {
        entity_id: u32 = 0,
        disabled: bool = true,
    }
    struct CreateReferredDamagePair(2004, 33) {
        source_entity_id: u32 = 0,
        target_entity_id: u32 = 0,
    }
    struct SetNetworkUpdateRate(2004, 34) {
        entity_id: u32 = 0,
        use_fixed_frequency: bool = true,
        update_frequency: i8 = 0, // enum: CharacterUpdateFrequency
    }
    struct ForceCharacterTreasure(2004, 37) {
        character_entity_id: u32 = 0,
    }
    struct BetrayCharactersCurrentCovenant(2004, 38) {
        dummy: u8 = 0,
    }
    struct WarpCharacterAndSetFloor(2004, 40) {
        entity_id: u32 = 0,
        warp_entity_type: u8 = 0, // enum: TargetEntityType
        warp_destination_entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        warp_destination_hit_entity_id: u32 = 0,
    }
    struct IssueShortWarpRequest(2004, 41) {
        entity_id: u32 = 0,
        warp_entity_type: u8 = 0, // enum: TargetEntityType
        warp_destination_entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
    }
    struct WarpCharacterAndCopyFloor(2004, 42) {
        entity_id: u32 = 0,
        warp_entity_type: u8 = 0, // enum: TargetEntityType
        warp_destination_entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        entity_id_to_copy_floor_from: u32 = 0,
    }
    struct RequestCharacterAnimationReset(2004, 43) {
        character_entity_id: u32 = 0,
        state: u8 = 0, // enum: DisabledEnabled
    }
    struct BonfireLikeRecovery(2004, 47) {}
    struct ChangeCharactersCloth(2004, 48) {
        entity_id: u32 = 0,
        bit_number: u8 = 0,
        state_id: u8 = 0,
    }
    struct ChangeCharacterPatrolBehavior(2004, 49) {
        character_entity_id: u32 = 0,
        patrol_information_entity_id: u32 = 0,
    }
    struct ChangeCharacter(2004, 52) {
        character_param_id: i32 = 0,
    }
    struct SetCharacterTalkRange(2004, 55) {
        entity_id: u32 = 0,
        range: f32 = 0f32,
    }
    struct ConnectCharacterToCaravan(2004, 60) {
        character_entity_id: u32 = 0,
        caravan_asset_entity_id: u32 = 0,
    }
    struct SetCharacterEnableDistance(2004, 63) {
        character_entity_id: u32 = 0,
        distance: f32 = 0f32,
    }
    struct CopyPlayerCharacterData(2004, 67) {
        source_character_entity_id: u32 = 0,
        target_character_entity_id: u32 = 0,
    }
    struct AttachAssetToCharacter(2004, 68) {
        character_entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        asset_entity_id: u32 = 0,
    }
    struct WarpCharacterAndCopyFloorWithFadeOut(2004, 74) {
        entity_id: u32 = 0,
        warp_entity_type: u8 = 0, // enum: TargetEntityType
        warp_destination_entity_id: u32 = 0,
        dummypoly_id: i32 = 0,
        entity_id_to_copy_floor_from: u32 = 0,
        use_bonfire_effect: bool = false,
        reset_camera: bool = false,
    }
    struct SetCharacterFaceParamOverride(2004, 75) {
        character_entity_id: u32 = 0,
        override_slot: i8 = 0,
        face_param_id: i32 = 0,
    }
    struct FadeToBlack(2004, 77) {
        fade_to_black_ratio: f32 = 0f32,
        fade_to_black_time_s: f32 = 0f32,
        freeze_player: bool = false,
        freeze_player_delay_s: f32 = 0f32,
    }
    struct CopyCharacterDataFromOnlinePlayers(2004, 78) {
        pool_type: i8 = 0,
        fallback_character_param_id: i32 = 0,
        target_entity_id: u32 = 0,
    }
    struct RequestCharacterDataFromOnlinePlayers(2004, 79) {
        pool_type: i8 = 0,
        unknown: i32 = 0,
    }
    struct SendCharacterDataToOnlinePlayers(2004, 80) {
        pool_type: i8 = 0,
    }
    struct ResetCharacterPosition(2004, 81) {
        character_entity_id: u32 = 0,
    }
    struct SetSpecialStandbyEndedFlag(2004, 83) {
        character_entity_id: u32 = 0,
        flag_state: u8 = 0, // enum: OnOff
    }
    struct FadeToColor(2004, 85) {
        opacity: f32 = 0f32,
        fade_time_s: f32 = 0f32,
        freeze_player: bool = false,
        freeze_player_delay_s: f32 = 0f32,
        red: f32 = 1f32,
        green: f32 = 1f32,
        blue: f32 = 1f32,
    }

    // Asset
    struct RequestAssetDestruction(2005, 1) {
        entity_id: u32 = 0,
        slot_number: i8 = 0,
    }
    struct RequestAssetRestoration(2005, 2) {
        entity_id: u32 = 0,
    }
    struct InitializeObjAct(2005, 5) {
        entity_id: u32 = 0,
        obj_act_param_id: i32 = -1,
        relative_target_idx: i32 = -1,
    }
    struct ReproduceAssetAnimation(2005, 7) {
        target_asset_entity_id: u32 = 0,
        animation_id: i32 = 0,
    }
    struct ReproduceAssetDestruction(2005, 8) {
        entity_id: u32 = 0,
        slot_number: i8 = 0,
    }
    struct CreateDamagingAsset(2005, 9) {
        event_flag_id: u32 = 0,
        entity_id: u32 = 0,
        dummypoly_id: i32 = 0,
        behavior_id: i32 = 0,
        target: i32 = 1, // enum: DamageTargetType
        radius: f32 = 0f32,
        lifespan: f32 = 0f32,
        repetition_time_s: f32 = 0f32,
    }
    struct WarpAssetToCharacter(2005, 11) {
        entity_id: u32 = 0,
        character_entity_id: u32 = 0,
        dummypoly_id: i16 = -1,
    }
    struct DeleteAssetEvent(2005, 12) {
        event_flag_id: u32 = 0,
    }
    struct RerollAssetTreasure(2005, 15) {
        target_asset_entity_id: u32 = 0,
    }
    struct AttachCaravanToController(2005, 17) {
        caravan_asset_entity_id: u32 = 0,
        character_entity_id: u32 = 0,
    }
    struct AttachAssetToAsset(2005, 18) {
        child_asset_entity_id: u32 = 0,
        parent_asset_entity_id: u32 = 0,
        parent_dummypoly_id: i32 = 0,
    }
    struct CreateBigDamagingAsset(2005, 20) {
        event_flag_id: u32 = 0,
        entity_id: u32 = 0,
        starting_dummypoly_id: i32 = 0,
        ending_dummypoly_id: i32 = 0,
        behavior_id: i32 = 0,
        target: i32 = 1, // enum: DamageTargetType
        radius: f32 = 0f32,
        lifespan: f32 = 0f32,
        repetition_time_s: f32 = 0f32,
    }

    // SFX
    struct DeleteMapSFX(2006, 1) {
        entity_id: u32 = 0,
        only_delete_root: bool = true,
    }
    struct SpawnMapSFX(2006, 2) {
        entity_id: u32 = 0,
    }
    struct SpawnOneshotSFX(2006, 3) {
        target_entity_type: i32 = 0, // enum: TargetEntityType
        entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        sfx_id: i32 = 0,
    }
    struct CreateAssetFollowingSFX(2006, 4) {
        asset_entity_id: u32 = 0,
        dummypoly_id: i32 = 0,
        sfx_id: i32 = 0,
    }
    struct DeleteAssetFollowingSFX(2006, 5) {
        asset_entity_id: u32 = 0,
        should_delete_root: bool = true,
    }
    struct SetWindSFX(2006, 6) {
        wind_sfx_id: i32 = 0,
    }

    // Message
    struct DisplayGenericDialog(2007, 1) {
        message_id: i32 = 0,
        dialog_type: i16 = 0, // enum: PromptType
        number_of_options: i16 = 6, // enum: NumberOfOptions
        entity_id: u32 = 0,
        display_distance: f32 = 0f32,
    }
    struct DisplayBanner(2007, 2) {
        banner_type: u8 = 8, // enum: TextBannerType
    }
    struct DisplayStatusMessage(2007, 3) {
        message_id: i32 = 0,
        pad_state: u8 = 0, // enum: DisabledEnabled
    }
    struct DisplayBlinkingMessage(2007, 4) {
        message_id: i32 = 0,
    }
    struct DisplayFullScreenMessage(2007, 9) {
        message_id: i32 = 0,
    }
    struct DisplayGenericDialogAndSetEventFlags(2007, 10) {
        message_id: i32 = 0,
        dialog_type: i16 = 0, // enum: PromptType
        number_of_options: i16 = 6, // enum: NumberOfOptions
        entity_id: u32 = 0,
        display_distance: f32 = 0f32,
        left_response_event_flag_id: u32 = 0,
        right_response_event_flag_id: u32 = 0,
        cancel_response_event_flag_id: u32 = 0,
    }
    struct DisplayBlinkingMessageWithPriority(2007, 12) {
        message_id: i32 = 0,
        priority: i16 = 0,
        should_interrupt: bool = false,
    }
    struct DisplaySubareaWelcomeMessage(2007, 13) {
        message_id: i32 = 0,
    }
    struct DisplayAreaWelcomeMessage(2007, 14) {
        message_id: i32 = 0,
    }
    struct ShowTutorialPopup(2007, 15) {
        tutorial_param_id: i32 = 0,
        unknown: bool = true,
        unknown2: bool = true,
    }
    struct DisplayNetworkMessage(2007, 16) {
        network_message_id: i32 = 0,
        unknown: bool = false,
    }

    // Camera
    struct ChangeCamera(2008, 1) {
        normal_camera_id: i32 = -1,
        locked_camera_id: i32 = -1,
    }
    struct SetCameraVibration(2008, 2) {
        vibration_id: i32 = 0,
        target_entity_type: i32 = 0, // enum: TargetEntityType
        entity_id: u32 = 0,
        dummypoly_id: i32 = -1,
        decay_start_distance: f32 = 0f32,
        decay_end_distance: f32 = 0f32,
    }
    struct SetLockcamSlotNumber(2008, 3) {
        area_id: u8 = 0,
        block_id: u8 = 0,
        lockcam_slot_number: u16 = 0,
    }
    struct SetCameraAngle(2008, 4) {
        x_angle: f32 = 0f32,
        y_angle: f32 = 0f32,
    }

    // Script
    struct RegisterLadder(2009, 0) {
        disable_top_event_flag_id: u32 = 0,
        disable_bottom_event_flag_id: u32 = 0,
        entity_id: u32 = 0,
    }
    struct RegisterBonfire(2009, 3) {
        event_flag_id: u32 = 0,
        entity_id: u32 = 0,
        reaction_distance: f32 = 0f32,
        reaction_angle: f32 = 0f32,
        set_standard_kindling_level: i32 = 0,
        enemy_deactivation_distance: f32 = 0f32,
    }
    struct ActivateMultiplayerDependantBuffs(2009, 4) {
        entity_id: u32 = 0,
    }
    struct IssueBossRoomEntryNotification(2009, 6) {
        dummy: u8 = 0,
    }
    struct SendInvadingPhantomsHome(2009, 8) {
        unknown: i32 = 0,
    }
    struct SendAllPhantomsHome(2009, 11) {
        dummy: i32 = 0,
    }
    struct SendAllPhantomsHomeAndUpdateServerPvpStats(2009, 12) {
        unknown: i32 = 0,
    }

    // Sound
    struct PlaySE(2010, 2) {
        entity_id: u32 = 0,
        sound_type: i32 = 0, // enum: SoundType
        sound_id: i32 = 0,
    }
    struct SuppressSE(2010, 7) {
        sound_type: i32 = 0, // enum: SoundType
        unknown: i32 = 0,
        is_suppression_active: bool = false,
    }
    struct SetBossBGM(2010, 10) {
        bgm_boss_conv_param_id: i32 = 0,
        state: i32 = 0, // enum: BossBGMState
    }
    struct SuppressSoundForFogGate(2010, 11) {
        time_s: f32 = 0f32,
    }
    struct SetFieldBattleBGMHeatUp(2010, 12) {
        npc_threat_level: u32 = 0,
        heat_up: bool = false,
    }

    // Map
    struct ActivateGparamOverride(2012, 11) {
        gparam_sub_id: i32 = 0,
        change_time_s: f32 = 0f32,
    }
    struct DeactivateGparamOverride(2012, 12) {
        change_time_s: f32 = 0f32,
    }

    // Enable/disable
    // System
    struct NetworkSync(2000, 2) {
        enabled: u8 = 0,
    }

    // Event
    struct Generator(2003, 3) {
        generator_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct MapHit(2003, 6) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct MapVisibility(2003, 7) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct MessageVisibility(2003, 26) {
        message_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct DirectionDisplay(2003, 44) {
        enabled: u8 = 0,
    }
    struct TextOnLoadingScreen(2003, 80) {
        enabled: u8 = 0,
    }

    // Character
    struct CharacterAI(2004, 1) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct Character(2004, 5) {
        character_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterGravity(2004, 10) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterImmortality(2004, 12) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterInvincibility(2004, 15) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterHPBarDisplay(2004, 30) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterDefaultBackread(2004, 35) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterCollision(2004, 39) {
        entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct LockOnPoint(2004, 50) {
        entity_id: u32 = 0,
        lock_on_dummypoly_id: i32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterDisableOnHitUnload(2004, 69) {
        character_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct DistanceBasedNetworkUpdateAuthority(2004, 70) {
        character_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct CharacterFadeOnEnable(2004, 73) {
        character_entity_id: u32 = 0,
        enabled: u8 = 0,
    }

    // Asset
    struct Asset(2005, 3) {
        target_asset_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct AssetTreasure(2005, 4) {
        target_asset_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct ObjAct(2005, 6) {
        entity_id: u32 = 0,
        obj_act_param_id: i32 = -1,
        enabled: u8 = 0,
    }
    struct AssetInvulnerability(2005, 13) {
        target_asset_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct ObjActAssignIdx(2005, 14) {
        entity_id: u32 = 0,
        obj_act_param_id: i32 = -1,
        relative_target_idx: i32 = -1,
        enabled: u8 = 0,
    }

    // Hit
    struct Hit(2011, 1) {
        hit_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct HitBackreadMask(2011, 2) {
        hit_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct Hitres(2011, 3) {
        hit_entity_id: u32 = 0,
        enabled: u8 = 0,
    }

    // Map
    struct MapPart(2012, 1) {
        map_part_entity_id: u32 = 0,
        enabled: u8 = 0,
    }
    struct AreaWelcomeMessage(2012, 8) {
        enabled: u8 = 0,
    }

}

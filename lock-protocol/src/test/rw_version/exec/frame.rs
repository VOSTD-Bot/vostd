use std::mem::ManuallyDrop;
use core::ops::Deref;

use builtin::*;
use builtin_macros::*;
use vstd::prelude::*;
use vstd::rwlock::*;
use vstd::vstd::arithmetic::power2::*;
use vstd::bits::*;
use vstd::atomic_ghost::AtomicU64;

use super::super::vstd_extra::{
    manually_drop::*,
};
use super::super::spec::{
    common::*,
    utils::*,
    tree::*,
};
use super::{
    common::*,
    types::*,
};

verus!{

pub type FrameId = u64;

pub const MAX_FRAME_NUM: u64 = 256;

pub open spec fn valid_fid(fid: FrameId) -> bool {
    0 <= fid < MAX_FRAME_NUM
}

pub open spec fn valid_paddr(pa: Paddr) -> bool {
    0 <= pa < (MAX_FRAME_NUM << 12)
}

pub open spec fn paddr_is_aligned_spec(pa: Paddr) -> bool {
    (pa & (low_bits_mask(12) as u64)) == 0
}

#[verifier::when_used_as_spec(paddr_is_aligned_spec)]
pub fn paddr_is_aligned(pa: Paddr) -> (res: bool)
    requires
        valid_paddr(pa),
    ensures
        res == paddr_is_aligned_spec(pa),
{
    assume(false);
    (pa & (1u64 << 12)) == 0
}

pub open spec fn fid_to_pa_spec(fid: FrameId) -> (res: Paddr) {
    fid << 12
}

#[verifier::when_used_as_spec(fid_to_pa_spec)]
pub fn fid_to_pa(fid: FrameId) -> (res: Paddr)
    requires
        valid_fid(fid),
    ensures
        res == fid_to_pa_spec(fid),
        valid_paddr(res),
        paddr_is_aligned(res),
{
    assume(false);
    fid << 12
}

pub open spec fn pa_to_fid_spec(pa: Paddr) -> FrameId {
    pa >> 12
}

#[verifier::when_used_as_spec(pa_to_fid_spec)]
pub fn pa_to_fid(pa: Paddr) -> (res: FrameId)
    requires
        valid_paddr(pa),
        paddr_is_aligned(pa),
    ensures
        res == pa_to_fid_spec(pa),
        valid_fid(res),
{
    assume(false);
    pa >> 12
}

}

verus!{

pub union Frame {
    pub void_frame: ManuallyDrop<VoidFrame>,
    pub page_table_frame: ManuallyDrop<PageTableFrame>,
}

pub struct VoidFrame {
    pub pa: Paddr,
}

struct_with_invariants!{

pub struct PageTableFrame {
    // Corresponding node id
    pub nid: Ghost<NodeId>,

    // Metadatas in metaslot
    pub pa: Paddr,
    pub rw_lock: RwLock<Tracked<NodeToken>, spec_fn(Tracked<NodeToken>) -> bool>,
    pub rc: AtomicU64<_, RcToken, _>,
    
    // // Actual contents in frame
    // pub ptes: Vec<AtomicU64<_, Option<()>, _>>,

    pub inst: Tracked<SpecInstance>,
}

pub open spec fn wf(&self) -> bool {
    predicate {
        &&& NodeHelper::valid_nid(self.nid@)

        &&& valid_paddr(self.pa)
        &&& forall |token: Tracked<NodeToken>| #[trigger] self.rw_lock.inv(token) <==> {
            &&& token@.instance_id() == self.inst@.id()
            &&& token@.key() == self.nid@
            &&& token@.value().is_WriteUnLocked()
        }

        // &&& self.ptes@.len() == 512

        &&& self.inst@.cpu_num() == GLOBAL_CPU_NUM
    }

    invariant on rc with (nid, inst) is (v: u64, g: RcToken) {
        &&& g.instance_id() == inst@.id()
        &&& g.key() == nid@
        &&& g.value() == v

        &&& v <= MAX_RC() // prevent overflow
    }

    // invariant on ptes
    //     forall |offset: nat| where (valid_pte_offset(offset))
    //     specifically (self.ptes@[offset as int])
    //     is (pa: u64, g: Option<()>)
    // {
    //     &&& pa != INVALID_PADDR <==> g.is_Some()
    // }
}

}

#[is_variant]
pub enum FrameUsage {
    Void,
    PageTable,
}

struct_with_invariants!{

pub struct FrameAllocator {
    pub frames: Vec<Frame>,
    pub usages: Vec<FrameUsage>,
}

pub open spec fn wf(&self) -> bool {
    predicate {
        &&& self.frames@.len() == MAX_FRAME_NUM
        &&& self.usages@.len() == MAX_FRAME_NUM

        &&& forall |fid: FrameId| valid_fid(fid) ==>
            match self.usages@[fid as int] {
                FrameUsage::Void => {
                    &&& is_variant(self.frames@[fid as int], "void_frame")
                },
                FrameUsage::PageTable => {
                    &&& is_variant(self.frames@[fid as int], "page_table_frame")
                    &&& get_union_field::<Frame, ManuallyDrop<PageTableFrame>>(
                        self.frames@[fid as int], 
                        "page_table_frame",
                    ).deref().wf()
                    &&& get_union_field::<Frame, ManuallyDrop<PageTableFrame>>(
                        self.frames@[fid as int], 
                        "page_table_frame",
                    ).deref().pa == fid_to_pa(fid)
                },
            }
    }
}

}

impl FrameAllocator {

    pub open spec fn inv_pt_frame(frame: Frame) -> bool {
        &&& is_variant(frame, "page_table_frame")
        &&& get_union_field::<Frame, ManuallyDrop<PageTableFrame>>(
            frame, 
            "page_table_frame",
        ).deref().wf()
    }

    pub open spec fn get_pt_frame_from_pa_spec(&self, pa: Paddr) -> PageTableFrame {
        *get_union_field::<Frame, ManuallyDrop<PageTableFrame>>(
            self.frames@[pa_to_fid(pa) as int],
            "page_table_frame",
        ).deref()
    }

    pub fn get_pt_frame_from_pa(&self, pa: Paddr) -> (res: &Frame)
        requires
            self.wf(),
            valid_paddr(pa),
            paddr_is_aligned(pa),
            self.usages@[pa_to_fid_spec(pa) as int].is_PageTable(),
        ensures
            *res =~= self.frames@[pa_to_fid(pa) as int],
            Self::inv_pt_frame(*res),
    {
        let fid: FrameId = pa_to_fid(pa);
        &self.frames[fid as usize]
    }

    #[verifier::external_body]
    pub fn find_void_frame(&self) -> (res: FrameId)
        requires
            self.wf(),
        ensures
            valid_fid(res),
            self.usages@[res as int].is_Void(),
    {
        0
    }

    pub fn find_void_frame_pa(&self) -> (res: Paddr)
        requires
            self.wf(),
        ensures
            valid_paddr(res),
            paddr_is_aligned(res),
            self.usages@[pa_to_fid(res) as int].is_Void(),
    {
        let fid = self.find_void_frame();
        assert(pa_to_fid(fid_to_pa(fid)) == fid) by { admit(); };
        fid_to_pa(fid)
    }

    #[verifier::external_body]
    pub fn allocate_pt_frame(
        // &mut self,
        &self,
        pa: Paddr,
        nid: Ghost<NodeId>,
        inst: Tracked<SpecInstance>,
        node_token: Tracked<NodeToken>,
        rc_token: Tracked<RcToken>,
    )
        requires
            self.wf(),
            valid_paddr(pa),
            paddr_is_aligned(pa),
            self.usages@[pa_to_fid(pa) as int].is_Void(), // Fix this

            NodeHelper::valid_nid(nid@),
            inst@.cpu_num() == GLOBAL_CPU_NUM,
            node_token@.instance_id() == inst@.id(),
            node_token@.key() == nid@,
            node_token@.value().is_WriteUnLocked(),
            rc_token@.instance_id() == inst@.id(),
            rc_token@.key() == nid@,
            rc_token@.value() == 0,
        ensures
            self.wf(),
            self.usages@[pa_to_fid(pa) as int].is_PageTable(),
    {}

}

}
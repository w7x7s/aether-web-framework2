//! # Radix Router with O(K) Lookup and Single-Instruction Bitmask Method Matching

use crate::node::{HandlerId, NodeType, RouteNode};
use crate::params::{Param, PathParams};
use aether_http::method::Method;

pub struct RadixRouter {
    root: RouteNode,
}

#[derive(Debug)]
pub enum MatchResult<'a> {
    Found {
        handler_id: HandlerId,
        params: PathParams<'a>,
    },
    MethodNotAllowed {
        allowed_bitmask: u64,
    },
    NotFound,
}

impl RadixRouter {
    pub fn new() -> Self {
        Self {
            root: RouteNode::new("/".to_string(), NodeType::Static),
        }
    }

    /// تسجيل مسار جديد داخل شجرة التوجيه (مثل `/api/v1/users/{id}` أو `/static/*filepath`).
    /// 
    /// يتم تحليل المقاطع وإنشاء العقد الفرعية المناسبة مع فرزها لضمان أسبقية المطابقة الحتمية.
    pub fn add_route(&mut self, method: Method, path: &str, handler: HandlerId) {
        let segments = path.trim_matches('/').split('/').filter(|s| !s.is_empty());
        let mut curr = &mut self.root;

        if path == "/" || path.is_empty() {
            curr.add_handler(method, handler);
            return;
        }

        for seg in segments {
            let (node_type, is_catchall) = if seg.starts_with('{') && seg.ends_with('}') {
                let name = &seg[1..seg.len() - 1];
                (NodeType::Param(name.to_string()), false)
            } else if seg.starts_with('*') {
                let name = if seg.len() > 1 { &seg[1..] } else { "wildcard" };
                (NodeType::CatchAll(name.to_string()), true)
            } else {
                (NodeType::Static, false)
            };

            let child_idx = curr.children.iter().position(|c| match (&c.node_type, &node_type) {
                (NodeType::Static, NodeType::Static) => c.prefix == seg,
                (NodeType::Param(p1), NodeType::Param(p2)) => p1 == p2,
                (NodeType::CatchAll(p1), NodeType::CatchAll(p2)) => p1 == p2,
                _ => false,
            });

            if let Some(idx) = child_idx {
                curr = &mut curr.children[idx];
            } else {
                let new_node = RouteNode::new(seg.to_string(), node_type);
                curr.children.push(new_node);

                // فرز العقد الفرعية لضمان الأسبقية الحتمية: Static أولاً، ثم Param، ثم CatchAll
                curr.children.sort_by_key(|c| match &c.node_type {
                    NodeType::Static => 0,
                    NodeType::Param(_) => 1,
                    NodeType::CatchAll(_) => 2,
                });

                let pos = curr.children.iter().position(|c| c.prefix == seg).unwrap();
                curr = &mut curr.children[pos];
            }

            if is_catchall {
                break;
            }
        }

        curr.add_handler(method, handler);
    }

    /// مطابقة المسار وأسلوب HTTP في زمن $O(K)$ حيث $K$ هو عمق المسار، مع صفر تخصيصات للذاكرة على الـ Heap (Zero-Alloc Stack Matching).
    pub fn find<'a>(&'a self, method: Method, path: &'a str) -> MatchResult<'a> {
        let mut params = PathParams::new();

        if path == "/" || path.is_empty() {
            if let Some(h) = self.root.get_handler(method) {
                return MatchResult::Found { handler_id: h, params };
            } else if self.root.allowed_methods != 0 {
                return MatchResult::MethodNotAllowed {
                    allowed_bitmask: self.root.allowed_methods,
                };
            } else {
                return MatchResult::NotFound;
            }
        }

        // استخدام SmallVec على الـ Stack لتفادي تخصيص مصفوفة على الـ Heap
        let mut segments: smallvec::SmallVec<[&'a str; 8]> = smallvec::SmallVec::new();
        for seg in path.trim_matches('/').split('/') {
            if !seg.is_empty() {
                segments.push(seg);
            }
        }
        self.match_recursive(&self.root, method, &segments, 0, &mut params)
    }

    fn match_recursive<'a>(
        &'a self,
        curr: &'a RouteNode,
        method: Method,
        segments: &[&'a str],
        index: usize,
        params: &mut PathParams<'a>,
    ) -> MatchResult<'a> {
        if index == segments.len() {
            if let Some(handler_id) = curr.get_handler(method) {
                return MatchResult::Found {
                    handler_id,
                    params: params.clone(),
                };
            } else if curr.allowed_methods != 0 {
                return MatchResult::MethodNotAllowed {
                    allowed_bitmask: curr.allowed_methods,
                };
            } else {
                return MatchResult::NotFound;
            }
        }

        let seg = segments[index];

        // Try static matches first
        for child in &curr.children {
            if let NodeType::Static = &child.node_type {
                if child.prefix == seg {
                    let res = self.match_recursive(child, method, segments, index + 1, params);
                    if !matches!(res, MatchResult::NotFound) {
                        return res;
                    }
                }
            }
        }

        // Try param matches second
        for child in &curr.children {
            if let NodeType::Param( key) = &child.node_type {
                params.push(Param {
                    key: key.as_str(),
                    value: seg,
                });
                let res = self.match_recursive(child, method, segments, index + 1, params);
                if !matches!(res, MatchResult::NotFound) {
                    return res;
                }
                params.pop();
            }
        }

        // Try catch-all matches last
        for child in &curr.children {
            if let NodeType::CatchAll( key) = &child.node_type {
                // Leak-free catch-all parameter borrowing first segment or full remaining
                params.push(Param {
                    key: key.as_str(),
                    value: seg,
                });
                if let Some(handler_id) = child.get_handler(method) {
                    return MatchResult::Found {
                        handler_id,
                        params: params.clone(),
                    };
                } else if child.allowed_methods != 0 {
                    return MatchResult::MethodNotAllowed {
                        allowed_bitmask: child.allowed_methods,
                    };
                }
                params.pop();
            }
        }

        MatchResult::NotFound
    }
}

impl Default for RadixRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_radix_routing_with_params_and_precedence() {
        let mut router = RadixRouter::new();
        router.add_route(Method::Get, "/", 10);
        router.add_route(Method::Get, "/users", 20);
        router.add_route(Method::Get, "/users/me", 25); // Exact static route
        router.add_route(Method::Get, "/users/{id}", 30); // Param route
        router.add_route(Method::Post, "/users/{id}/posts/{post_id}", 40);

        // Test root
        match router.find(Method::Get, "/") {
            MatchResult::Found { handler_id, .. } => assert_eq!(handler_id, 10),
            _ => panic!("Expected root"),
        }

        // Test static precedence over dynamic parameter (/users/me must resolve to 25, not 30)
        match router.find(Method::Get, "/users/me") {
            MatchResult::Found { handler_id, params } => {
                assert_eq!(handler_id, 25);
                assert_eq!(params.len(), 0);
            }
            _ => panic!("Expected /users/me to match static route with precedence"),
        }

        // Test param extraction without allocation
        match router.find(Method::Get, "/users/42") {
            MatchResult::Found { handler_id, params } => {
                assert_eq!(handler_id, 30);
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].key, "id");
                assert_eq!(params[0].value, "42");
            }
            _ => panic!("Expected user 42"),
        }

        // Test multi-param
        match router.find(Method::Post, "/users/alice/posts/101") {
            MatchResult::Found { handler_id, params } => {
                assert_eq!(handler_id, 40);
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].value, "alice");
                assert_eq!(params[1].value, "101");
            }
            _ => panic!("Expected post 101"),
        }

        // Test 405 Method Not Allowed
        match router.find(Method::Delete, "/users/42") {
            MatchResult::MethodNotAllowed { allowed_bitmask } => {
                assert!(allowed_bitmask & Method::Get.bitmask() != 0);
            }
            _ => panic!("Expected 405"),
        }
    }
}
